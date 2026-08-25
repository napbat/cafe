//! JVM instruction and operand lowering into shared disassembly IR.

use disassembler::{
    CodeAddress, CodeSize, ExactText, ExceptionBehavior, Immediate,
    Instruction as SharedInstruction, InstructionFlow, Operand as SharedOperand, Reference,
    ReferenceKind, ReferenceSymbol, SwitchCase, SwitchTable,
};

use crate::bytecode::{Instruction, Operand};
use crate::classfile::{Constant, ConstantPool};
use crate::{Error, Result};

pub(super) fn lower_instruction(
    instruction: &Instruction,
    pool: &ConstantPool,
) -> Result<SharedInstruction> {
    let address = address_from_usize(instruction.offset)?;
    let size = size_from_usize(instruction.size, instruction.offset)?;
    let operands = lower_operand(instruction, pool)?;
    let flow = lower_flow(instruction)?;

    Ok(SharedInstruction::new(
        address,
        size,
        u32::from(instruction.opcode.byte()),
        instruction.mnemonic(),
        operands,
        flow,
    )
    .with_exception_behavior(exception_behavior(instruction.opcode)))
}

fn exception_behavior(opcode: crate::bytecode::Opcode) -> ExceptionBehavior {
    use crate::bytecode::Opcode;

    if matches!(
        opcode,
        Opcode::Breakpoint | Opcode::ImpDep1 | Opcode::ImpDep2
    ) {
        return ExceptionBehavior::Unknown;
    }
    ExceptionBehavior::from_may_throw(matches!(
        opcode,
        Opcode::Ldc
            | Opcode::LdcW
            | Opcode::Ldc2W
            | Opcode::IALoad
            | Opcode::LALoad
            | Opcode::FALoad
            | Opcode::DALoad
            | Opcode::AALoad
            | Opcode::BALoad
            | Opcode::CALoad
            | Opcode::SALoad
            | Opcode::IAStore
            | Opcode::LAStore
            | Opcode::FAStore
            | Opcode::DAStore
            | Opcode::AAStore
            | Opcode::BAStore
            | Opcode::CAStore
            | Opcode::SAStore
            | Opcode::IDiv
            | Opcode::LDiv
            | Opcode::IRem
            | Opcode::LRem
            | Opcode::IReturn
            | Opcode::LReturn
            | Opcode::FReturn
            | Opcode::DReturn
            | Opcode::AReturn
            | Opcode::Return
            | Opcode::GetStatic
            | Opcode::PutStatic
            | Opcode::GetField
            | Opcode::PutField
            | Opcode::InvokeVirtual
            | Opcode::InvokeSpecial
            | Opcode::InvokeStatic
            | Opcode::InvokeInterface
            | Opcode::InvokeDynamic
            | Opcode::New
            | Opcode::NewArray
            | Opcode::ANewArray
            | Opcode::ArrayLength
            | Opcode::AThrow
            | Opcode::CheckCast
            | Opcode::InstanceOf
            | Opcode::MonitorEnter
            | Opcode::MonitorExit
            | Opcode::MultiANewArray
    ))
}

fn lower_operand(instruction: &Instruction, pool: &ConstantPool) -> Result<Vec<SharedOperand>> {
    let operands = match &instruction.operand {
        Operand::None => Vec::new(),
        Operand::Byte(value) => vec![signed(i64::from(*value))],
        Operand::Short(value) => vec![signed(i64::from(*value))],
        Operand::Constant(index) | Operand::InvokeDynamic(index) => {
            vec![lower_reference(*index, pool)?]
        }
        Operand::Local(index) => vec![SharedOperand::Local(u32::from(*index))],
        Operand::Increment { index, value } => vec![
            SharedOperand::Local(u32::from(*index)),
            signed(i64::from(*value)),
        ],
        Operand::Branch(target) => vec![SharedOperand::BranchTarget(address_from_target(
            *target,
            instruction.offset,
        )?)],
        Operand::TableSwitch { .. } | Operand::LookupSwitch { .. } => {
            vec![SharedOperand::Switch(lower_switch(
                &instruction.operand,
                instruction.offset,
            )?)]
        }
        Operand::ArrayType(array_type) => {
            vec![SharedOperand::TypeName(array_type.name().to_owned())]
        }
        Operand::InvokeInterface { index, count } => {
            vec![lower_reference(*index, pool)?, unsigned(u64::from(*count))]
        }
        Operand::MultiArray { index, dimensions } => vec![
            lower_reference(*index, pool)?,
            unsigned(u64::from(*dimensions)),
        ],
    };
    Ok(operands)
}

fn lower_flow(instruction: &Instruction) -> Result<InstructionFlow> {
    use crate::bytecode::Opcode;

    let flow = match instruction.opcode {
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
        | Opcode::IfNull
        | Opcode::IfNonNull => InstructionFlow::ConditionalBranch {
            target: branch_target(instruction)?,
        },
        Opcode::Goto | Opcode::GotoW => InstructionFlow::UnconditionalBranch {
            target: branch_target(instruction)?,
        },
        Opcode::Jsr | Opcode::JsrW => InstructionFlow::SubroutineCall {
            target: branch_target(instruction)?,
        },
        Opcode::Ret => InstructionFlow::IndirectBranch,
        Opcode::TableSwitch | Opcode::LookupSwitch => {
            let table = lower_switch(&instruction.operand, instruction.offset)?;
            InstructionFlow::Switch {
                default: table.default,
                cases: table.cases,
            }
        }
        Opcode::IReturn
        | Opcode::LReturn
        | Opcode::FReturn
        | Opcode::DReturn
        | Opcode::AReturn
        | Opcode::Return => InstructionFlow::Return,
        Opcode::AThrow => InstructionFlow::Throw,
        _ => InstructionFlow::FallThrough,
    };
    Ok(flow)
}

fn branch_target(instruction: &Instruction) -> Result<CodeAddress> {
    if let Operand::Branch(target) = instruction.operand {
        address_from_target(target, instruction.offset)
    } else {
        Err(Error::invalid_bytecode(
            instruction.offset,
            format!(
                "{} has a non-branch operand in decoded instruction IR",
                instruction.mnemonic()
            ),
        ))
    }
}

fn lower_switch(operand: &Operand, source: usize) -> Result<SwitchTable> {
    match operand {
        Operand::TableSwitch {
            default,
            low,
            targets,
        } => {
            let cases = targets
                .iter()
                .enumerate()
                .map(|(position, target)| {
                    let position = i64::try_from(position).map_err(|_| {
                        Error::invalid_bytecode(source, "switch case position exceeds i64")
                    })?;
                    Ok(SwitchCase {
                        key: i64::from(*low) + position,
                        target: address_from_target(*target, source)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(SwitchTable {
                default: address_from_target(*default, source)?,
                cases,
            })
        }
        Operand::LookupSwitch { default, pairs } => Ok(SwitchTable {
            default: address_from_target(*default, source)?,
            cases: pairs
                .iter()
                .map(|(key, target)| {
                    Ok(SwitchCase {
                        key: i64::from(*key),
                        target: address_from_target(*target, source)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        }),
        _ => Err(Error::invalid_bytecode(
            source,
            "switch opcode has a non-switch operand in decoded instruction IR",
        )),
    }
}

fn lower_reference(index: u16, pool: &ConstantPool) -> Result<SharedOperand> {
    let kind = match pool.get(index)? {
        Constant::String { .. } => ReferenceKind::String,
        Constant::Class { .. } => ReferenceKind::Type,
        Constant::FieldRef { .. } => ReferenceKind::Field,
        Constant::MethodRef { .. } => ReferenceKind::Method,
        Constant::InterfaceMethodRef { .. } => ReferenceKind::InterfaceMethod,
        Constant::InvokeDynamic { .. } => ReferenceKind::DynamicCallSite,
        _ => ReferenceKind::Constant,
    };
    let reference = Reference::resolved(kind, u32::from(index), pool.describe(index)?);
    Ok(SharedOperand::Reference(
        reference_symbol(index, pool)?
            .map_or(reference.clone(), |symbol| reference.with_symbol(symbol)),
    ))
}

fn reference_symbol(index: u16, pool: &ConstantPool) -> Result<Option<ReferenceSymbol>> {
    let symbol = match pool.get(index)? {
        Constant::Integer(value) => Some(ReferenceSymbol::Integer(*value)),
        Constant::Float(value) => Some(ReferenceSymbol::Float(value.to_bits())),
        Constant::Long(value) => Some(ReferenceSymbol::Long(*value)),
        Constant::Double(value) => Some(ReferenceSymbol::Double(value.to_bits())),
        Constant::String { string_index } => {
            Some(ReferenceSymbol::String(exact_utf8(pool, *string_index)?))
        }
        Constant::Class { .. } => Some(ReferenceSymbol::Type(pool.class_name(index)?.to_owned())),
        Constant::FieldRef {
            class_index,
            name_and_type_index,
        } => Some(member_symbol(
            pool,
            *class_index,
            *name_and_type_index,
            false,
        )?),
        Constant::MethodRef {
            class_index,
            name_and_type_index,
        }
        | Constant::InterfaceMethodRef {
            class_index,
            name_and_type_index,
        } => Some(member_symbol(
            pool,
            *class_index,
            *name_and_type_index,
            true,
        )?),
        Constant::MethodType { descriptor_index } => Some(ReferenceSymbol::MethodPrototype(
            pool.utf8(*descriptor_index)?.to_owned(),
        )),
        Constant::Unusable
        | Constant::Utf8(_)
        | Constant::NameAndType { .. }
        | Constant::MethodHandle { .. }
        | Constant::Dynamic { .. }
        | Constant::InvokeDynamic { .. }
        | Constant::Module { .. }
        | Constant::Package { .. } => None,
    };
    Ok(symbol)
}

fn member_symbol(
    pool: &ConstantPool,
    class_index: u16,
    name_and_type_index: u16,
    method: bool,
) -> Result<ReferenceSymbol> {
    let Constant::NameAndType {
        name_index,
        descriptor_index,
    } = pool.get(name_and_type_index)?
    else {
        return Err(Error::invalid_bytecode(
            0,
            "member reference lacks a NameAndType constant",
        ));
    };
    let owner = pool.class_name(class_index)?.to_owned();
    let name = exact_utf8(pool, *name_index)?;
    let descriptor = pool.utf8(*descriptor_index)?.to_owned();
    Ok(if method {
        ReferenceSymbol::Method {
            owner,
            name,
            descriptor,
        }
    } else {
        ReferenceSymbol::Field {
            owner,
            name,
            descriptor,
        }
    })
}

fn exact_utf8(pool: &ConstantPool, index: u16) -> Result<ExactText> {
    let Constant::Utf8(value) = pool.get(index)? else {
        return Err(Error::invalid_bytecode(
            0,
            "symbolic reference text is not a Utf8 constant",
        ));
    };
    Ok(ExactText {
        text: value.as_str().to_owned(),
        utf16_units: value.utf16_units().to_vec(),
    })
}

fn signed(value: i64) -> SharedOperand {
    SharedOperand::Immediate(Immediate::Signed(value))
}

fn unsigned(value: u64) -> SharedOperand {
    SharedOperand::Immediate(Immediate::Unsigned(value))
}

fn address_from_target(target: i32, source: usize) -> Result<CodeAddress> {
    let target = u32::try_from(target).map_err(|_| {
        Error::invalid_bytecode(source, format!("negative absolute branch target {target}"))
    })?;
    Ok(CodeAddress::from(target))
}

fn address_from_usize(value: usize) -> Result<CodeAddress> {
    u64::try_from(value).map(CodeAddress::new).map_err(|_| {
        Error::invalid_bytecode(
            value,
            "instruction address does not fit shared address type",
        )
    })
}

fn size_from_usize(value: usize, source: usize) -> Result<CodeSize> {
    u32::try_from(value).map(CodeSize::new).map_err(|_| {
        Error::invalid_bytecode(source, "instruction size does not fit shared size type")
    })
}
