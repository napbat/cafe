//! JVM instruction and operand lowering into shared disassembly IR.

use disassembler::{
    CodeAddress, CodeSize, Immediate, Instruction as SharedInstruction, InstructionFlow,
    Operand as SharedOperand, Reference, ReferenceKind, SwitchCase, SwitchTable,
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
    Ok(SharedOperand::Reference(Reference::resolved(
        kind,
        u32::from(index),
        pool.describe(index)?,
    )))
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
