//! Reverse lowering from shared JVM instructions to typed native bytecode.

use disassembler::{
    CodeAddress, FunctionBody, Immediate, Instruction as SharedInstruction,
    Operand as SharedOperand, Reference,
};

use super::{JavaEmissionError, JavaReferenceResolver};
use crate::bytecode::{ArrayType, Instruction, Opcode, Operand};
use crate::classfile::ConstantPool;

pub(super) fn lower_instructions<R: JavaReferenceResolver>(
    body: &FunctionBody,
    class: &str,
    method: &str,
    descriptor: &str,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<Vec<Instruction>, JavaEmissionError> {
    body.instructions
        .iter()
        .map(|instruction| {
            lower_instruction(instruction, class, method, descriptor, pool, resolver)
                .map_err(|error| scope_instruction_error(error, class, method, descriptor))
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn lower_instruction<R: JavaReferenceResolver>(
    instruction: &SharedInstruction,
    class: &str,
    method: &str,
    descriptor: &str,
    pool: &mut ConstantPool,
    resolver: &mut R,
) -> Result<Instruction, JavaEmissionError> {
    let opcode_byte = u8::try_from(instruction.opcode)
        .ok()
        .and_then(Opcode::from_byte)
        .ok_or_else(|| invalid(instruction, class, method, descriptor, "unknown JVM opcode"))?;
    let offset = usize::try_from(instruction.address.get()).map_err(|_| {
        invalid(
            instruction,
            class,
            method,
            descriptor,
            "address does not fit the host bytecode offset type",
        )
    })?;
    let resolve = |reference: &Reference,
                   pool: &mut ConstantPool,
                   resolver: &mut R|
     -> Result<u16, JavaEmissionError> {
        resolver
            .resolve(reference, pool)
            .map_err(|source| JavaEmissionError::Reference {
                class: class.to_owned(),
                method: method.to_owned(),
                descriptor: descriptor.to_owned(),
                address: instruction.address,
                index: reference.index,
                source,
            })
    };
    let operand = match opcode_byte {
        Opcode::BiPush => Operand::Byte(i8::try_from(signed(instruction, 0)?).map_err(|_| {
            invalid(
                instruction,
                class,
                method,
                descriptor,
                "bipush value exceeds i8",
            )
        })?),
        Opcode::SiPush => Operand::Short(i16::try_from(signed(instruction, 0)?).map_err(|_| {
            invalid(
                instruction,
                class,
                method,
                descriptor,
                "sipush value exceeds i16",
            )
        })?),
        Opcode::Ldc
        | Opcode::LdcW
        | Opcode::Ldc2W
        | Opcode::GetStatic
        | Opcode::PutStatic
        | Opcode::GetField
        | Opcode::PutField
        | Opcode::InvokeVirtual
        | Opcode::InvokeSpecial
        | Opcode::InvokeStatic
        | Opcode::New
        | Opcode::ANewArray
        | Opcode::CheckCast
        | Opcode::InstanceOf => {
            Operand::Constant(resolve(reference(instruction, 0)?, pool, resolver)?)
        }
        Opcode::ILoad
        | Opcode::LLoad
        | Opcode::FLoad
        | Opcode::DLoad
        | Opcode::ALoad
        | Opcode::IStore
        | Opcode::LStore
        | Opcode::FStore
        | Opcode::DStore
        | Opcode::AStore
        | Opcode::Ret => Operand::Local(u16::try_from(local(instruction, 0)?).map_err(|_| {
            invalid(
                instruction,
                class,
                method,
                descriptor,
                "local index exceeds u16",
            )
        })?),
        Opcode::IInc => Operand::Increment {
            index: u16::try_from(local(instruction, 0)?).map_err(|_| {
                invalid(
                    instruction,
                    class,
                    method,
                    descriptor,
                    "local index exceeds u16",
                )
            })?,
            value: i16::try_from(signed(instruction, 1)?).map_err(|_| {
                invalid(
                    instruction,
                    class,
                    method,
                    descriptor,
                    "iinc value exceeds i16",
                )
            })?,
        },
        Opcode::IfEq
        | Opcode::IfNe
        | Opcode::IfLt
        | Opcode::IfGe
        | Opcode::IfGt
        | Opcode::IfLe
        | Opcode::IfICmpEq
        | Opcode::IfICmpNe
        | Opcode::IfICmpLt
        | Opcode::IfICmpGe
        | Opcode::IfICmpGt
        | Opcode::IfICmpLe
        | Opcode::IfACmpEq
        | Opcode::IfACmpNe
        | Opcode::Goto
        | Opcode::Jsr
        | Opcode::IfNull
        | Opcode::IfNonNull
        | Opcode::GotoW
        | Opcode::JsrW => Operand::Branch(i32_target(instruction, 0)?),
        Opcode::TableSwitch => table_switch(instruction)?,
        Opcode::LookupSwitch => lookup_switch(instruction)?,
        Opcode::InvokeInterface => Operand::InvokeInterface {
            index: resolve(reference(instruction, 0)?, pool, resolver)?,
            count: u8::try_from(unsigned(instruction, 1)?).map_err(|_| {
                invalid(
                    instruction,
                    class,
                    method,
                    descriptor,
                    "invokeinterface count exceeds u8",
                )
            })?,
        },
        Opcode::InvokeDynamic => {
            Operand::InvokeDynamic(resolve(reference(instruction, 0)?, pool, resolver)?)
        }
        Opcode::NewArray => Operand::ArrayType(array_type(instruction)?),
        Opcode::MultiANewArray => Operand::MultiArray {
            index: resolve(reference(instruction, 0)?, pool, resolver)?,
            dimensions: u8::try_from(unsigned(instruction, 1)?).map_err(|_| {
                invalid(
                    instruction,
                    class,
                    method,
                    descriptor,
                    "multianewarray dimensions exceed u8",
                )
            })?,
        },
        _ => {
            require_operand_count(instruction, 0)?;
            Operand::None
        }
    };
    let wide = matches!(
        operand,
        Operand::Local(index) if index > u16::from(u8::MAX)
    ) || matches!(
        operand,
        Operand::Increment { index, value }
            if index > u16::from(u8::MAX) || !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&value)
    );
    let mut native = if wide {
        Instruction::new_wide(offset, opcode_byte, operand)
    } else {
        Instruction::new(offset, opcode_byte, operand)
    };
    native.size = usize::try_from(instruction.size.get()).map_err(|_| {
        invalid(
            instruction,
            class,
            method,
            descriptor,
            "encoded size does not fit usize",
        )
    })?;
    Ok(native)
}

fn signed(instruction: &SharedInstruction, position: usize) -> Result<i64, JavaEmissionError> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Immediate(Immediate::Signed(value))) => Ok(*value),
        Some(SharedOperand::Immediate(Immediate::Unsigned(value))) => i64::try_from(*value)
            .map_err(|_| operand_error(instruction, "unsigned immediate exceeds i64")),
        _ => Err(operand_error(
            instruction,
            "expected signed immediate operand",
        )),
    }
}

fn unsigned(instruction: &SharedInstruction, position: usize) -> Result<u64, JavaEmissionError> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Immediate(Immediate::Unsigned(value))) => Ok(*value),
        Some(SharedOperand::Immediate(Immediate::Signed(value))) => u64::try_from(*value)
            .map_err(|_| operand_error(instruction, "negative unsigned immediate")),
        _ => Err(operand_error(
            instruction,
            "expected unsigned immediate operand",
        )),
    }
}

fn local(instruction: &SharedInstruction, position: usize) -> Result<u32, JavaEmissionError> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Local(index)) => Ok(*index),
        _ => Err(operand_error(
            instruction,
            "expected local-variable operand",
        )),
    }
}

fn reference(
    instruction: &SharedInstruction,
    position: usize,
) -> Result<&Reference, JavaEmissionError> {
    match instruction.operands.get(position) {
        Some(SharedOperand::Reference(reference)) => Ok(reference),
        _ => Err(operand_error(
            instruction,
            "expected symbolic reference operand",
        )),
    }
}

fn target(
    instruction: &SharedInstruction,
    position: usize,
) -> Result<CodeAddress, JavaEmissionError> {
    match instruction.operands.get(position) {
        Some(SharedOperand::BranchTarget(target)) => Ok(*target),
        _ => Err(operand_error(instruction, "expected branch-target operand")),
    }
}

fn i32_target(instruction: &SharedInstruction, position: usize) -> Result<i32, JavaEmissionError> {
    i32::try_from(target(instruction, position)?.get())
        .map_err(|_| operand_error(instruction, "branch target exceeds i32"))
}

fn table_switch(instruction: &SharedInstruction) -> Result<Operand, JavaEmissionError> {
    let table = switch(instruction)?;
    let Some(first) = table.cases.first() else {
        return Err(operand_error(
            instruction,
            "tableswitch requires at least one case",
        ));
    };
    let low = i32::try_from(first.key)
        .map_err(|_| operand_error(instruction, "tableswitch key exceeds i32"))?;
    let mut targets = Vec::with_capacity(table.cases.len());
    for (position, case) in table.cases.iter().enumerate() {
        let expected = i64::from(low)
            + i64::try_from(position)
                .map_err(|_| operand_error(instruction, "tableswitch case count exceeds i64"))?;
        if case.key != expected {
            return Err(operand_error(
                instruction,
                "tableswitch keys are not contiguous",
            ));
        }
        targets.push(
            i32::try_from(case.target.get())
                .map_err(|_| operand_error(instruction, "switch target exceeds i32"))?,
        );
    }
    Ok(Operand::TableSwitch {
        default: i32::try_from(table.default.get())
            .map_err(|_| operand_error(instruction, "switch default exceeds i32"))?,
        low,
        targets,
    })
}

fn lookup_switch(instruction: &SharedInstruction) -> Result<Operand, JavaEmissionError> {
    let table = switch(instruction)?;
    let pairs = table
        .cases
        .iter()
        .map(|case| {
            Ok((
                i32::try_from(case.key)
                    .map_err(|_| operand_error(instruction, "lookupswitch key exceeds i32"))?,
                i32::try_from(case.target.get())
                    .map_err(|_| operand_error(instruction, "switch target exceeds i32"))?,
            ))
        })
        .collect::<Result<Vec<_>, JavaEmissionError>>()?;
    Ok(Operand::LookupSwitch {
        default: i32::try_from(table.default.get())
            .map_err(|_| operand_error(instruction, "switch default exceeds i32"))?,
        pairs,
    })
}

fn switch(
    instruction: &SharedInstruction,
) -> Result<&disassembler::SwitchTable, JavaEmissionError> {
    require_operand_count(instruction, 1)?;
    match &instruction.operands[0] {
        SharedOperand::Switch(table) => Ok(table),
        _ => Err(operand_error(instruction, "expected switch-table operand")),
    }
}

fn array_type(instruction: &SharedInstruction) -> Result<ArrayType, JavaEmissionError> {
    require_operand_count(instruction, 1)?;
    let SharedOperand::TypeName(name) = &instruction.operands[0] else {
        return Err(operand_error(
            instruction,
            "expected primitive-array type name",
        ));
    };
    ArrayType::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.name() == name)
        .ok_or_else(|| operand_error(instruction, "unknown primitive-array type name"))
}

fn require_operand_count(
    instruction: &SharedInstruction,
    expected: usize,
) -> Result<(), JavaEmissionError> {
    if instruction.operands.len() == expected {
        Ok(())
    } else {
        Err(operand_error(
            instruction,
            format!(
                "expected {expected} operands, found {}",
                instruction.operands.len()
            ),
        ))
    }
}

fn operand_error(instruction: &SharedInstruction, message: impl Into<String>) -> JavaEmissionError {
    JavaEmissionError::Instruction {
        class: "<pending>".to_owned(),
        method: "<pending>".to_owned(),
        descriptor: String::new(),
        address: instruction.address,
        message: message.into(),
    }
}

fn invalid(
    instruction: &SharedInstruction,
    class: &str,
    method: &str,
    descriptor: &str,
    message: impl Into<String>,
) -> JavaEmissionError {
    JavaEmissionError::Instruction {
        class: class.to_owned(),
        method: method.to_owned(),
        descriptor: descriptor.to_owned(),
        address: instruction.address,
        message: message.into(),
    }
}

fn scope_instruction_error(
    error: JavaEmissionError,
    class: &str,
    method: &str,
    descriptor: &str,
) -> JavaEmissionError {
    match error {
        JavaEmissionError::Instruction {
            class: pending,
            method: _,
            descriptor: _,
            address,
            message,
        } if pending == "<pending>" => JavaEmissionError::Instruction {
            class: class.to_owned(),
            method: method.to_owned(),
            descriptor: descriptor.to_owned(),
            address,
            message,
        },
        error => error,
    }
}
