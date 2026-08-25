//! Recovery of Java's mandatory first constructor invocation.

use std::collections::{BTreeMap, BTreeSet};

use disassembler::ReferenceSymbol;
use java::descriptor::{JavaType, parse_method};
use mlil::{CallKind, Function, InstructionId, Operation, ValueType, VariableId, VariableRole};

use crate::names::SourceNames;

use super::instruction::{RenderFailure, constant_expression};

pub(super) struct ConstructorPrelude {
    pub(super) source: String,
    pub(super) instruction: InstructionId,
    pub(super) skipped: BTreeSet<InstructionId>,
}

#[allow(clippy::too_many_lines)]
pub(super) fn recover(
    function: &Function,
    owner: &str,
    parameters: &[JavaType],
    parameter_names: &[String],
    names: &SourceNames,
) -> Result<ConstructorPrelude, RenderFailure> {
    let mut values = initial_values(function, parameters, parameter_names);
    let mut skipped = BTreeSet::new();
    for block in function.cfg().reverse_postorder() {
        for instruction in function.cfg().block(block).instructions() {
            match instruction.operation() {
                Operation::Constant(constant) => {
                    values.insert(instruction.defs()[0], constant_expression(constant, names)?);
                }
                Operation::Copy => {
                    let value = values.get(&instruction.uses()[0]).cloned().ok_or_else(|| {
                        RenderFailure {
                            message: "constructor argument copy has no source expression"
                                .to_owned(),
                        }
                    })?;
                    values.insert(instruction.defs()[0], value);
                }
                Operation::ParallelCopy => {
                    let staged = instruction
                        .uses()
                        .iter()
                        .map(|variable| values.get(variable).cloned())
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| RenderFailure {
                            message: "constructor stack permutation has no source expression"
                                .to_owned(),
                        })?;
                    for (&definition, value) in instruction.defs().iter().zip(staged) {
                        values.insert(definition, value);
                    }
                }
                Operation::Call {
                    kind: CallKind::Direct | CallKind::Super,
                    target,
                    descriptor: Some(descriptor),
                } => {
                    let Some(ReferenceSymbol::Method {
                        owner: target_owner,
                        name,
                        ..
                    }) = &target.symbol
                    else {
                        return Err(RenderFailure {
                            message: "constructor invocation lacks a structured target".to_owned(),
                        });
                    };
                    if name.text != "<init>" {
                        return Err(RenderFailure {
                            message: "constructor executes a direct call before initialization"
                                .to_owned(),
                        });
                    }
                    let receiver = instruction
                        .uses()
                        .first()
                        .and_then(|variable| values.get(variable))
                        .ok_or_else(|| RenderFailure {
                            message: "constructor receiver has no source expression".to_owned(),
                        })?;
                    if receiver != "this" {
                        return Err(RenderFailure {
                            message: "constructor does not initialize its incoming receiver"
                                .to_owned(),
                        });
                    }
                    let descriptor = parse_method(descriptor).map_err(|error| RenderFailure {
                        message: error.to_string(),
                    })?;
                    let arguments = instruction.uses()[1..]
                        .iter()
                        .zip(&descriptor.parameters)
                        .map(|(variable, parameter)| {
                            values
                                .get(variable)
                                .map(|value| coerce_parameter(value, parameter))
                                .ok_or_else(|| RenderFailure {
                                    message: "constructor argument has no source expression"
                                        .to_owned(),
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .join(", ");
                    skipped.insert(instruction.id());
                    let invocation = if same_type(target_owner, owner) {
                        "this"
                    } else {
                        "super"
                    };
                    return Ok(ConstructorPrelude {
                        source: format!("{invocation}({arguments});"),
                        instruction: instruction.id(),
                        skipped,
                    });
                }
                Operation::Nop | Operation::Discard => {}
                _ => {
                    return Err(RenderFailure {
                        message: format!(
                            "{} cannot appear before a Java constructor invocation",
                            instruction.operation().mnemonic()
                        ),
                    });
                }
            }
            skipped.insert(instruction.id());
        }
    }
    Err(RenderFailure {
        message: "constructor has no initialization call".to_owned(),
    })
}

pub(super) fn fallback_invocation(
    function: &Function,
    owner: &str,
    names: &SourceNames,
) -> Option<String> {
    for block in function.cfg().reverse_postorder() {
        for instruction in function.cfg().block(block).instructions() {
            let Operation::Call {
                kind: CallKind::Direct | CallKind::Super,
                target,
                descriptor: Some(descriptor),
            } = instruction.operation()
            else {
                continue;
            };
            if !matches!(
                instruction.use_types().first(),
                Some(ValueType::UninitializedThis(_))
            ) {
                continue;
            }
            let Some(ReferenceSymbol::Method {
                owner: target_owner,
                name,
                ..
            }) = &target.symbol
            else {
                continue;
            };
            if name.text != "<init>" {
                continue;
            }
            let descriptor = parse_method(descriptor).ok()?;
            let arguments = descriptor
                .parameters
                .iter()
                .map(|parameter| default_argument(parameter, names))
                .collect::<Vec<_>>()
                .join(", ");
            let invocation = if same_type(target_owner, owner) {
                "this"
            } else {
                "super"
            };
            return Some(format!("{invocation}({arguments});"));
        }
    }
    None
}

fn initial_values(
    function: &Function,
    parameters: &[JavaType],
    parameter_names: &[String],
) -> BTreeMap<VariableId, String> {
    let mut values = BTreeMap::new();
    for variable in function.variables() {
        let VariableRole::Parameter(ordinal) = variable.role else {
            continue;
        };
        if ordinal == 0 {
            values.insert(variable.id, "this".to_owned());
            continue;
        }
        let index = usize::from(ordinal - 1);
        if parameters.get(index).is_some() {
            values.insert(
                variable.id,
                parameter_names
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| format!("parameter{index}")),
            );
        }
    }
    values
}

fn coerce_parameter(expression: &str, parameter: &JavaType) -> String {
    match parameter {
        JavaType::Boolean => format!("({expression} != 0)"),
        JavaType::Byte => format!("(byte) {expression}"),
        JavaType::Char => format!("(char) {expression}"),
        JavaType::Short => format!("(short) {expression}"),
        _ => expression.to_owned(),
    }
}

fn default_argument(value: &JavaType, names: &SourceNames) -> String {
    match value {
        JavaType::Boolean => "false".to_owned(),
        JavaType::Byte => "(byte) 0".to_owned(),
        JavaType::Char => "(char) 0".to_owned(),
        JavaType::Short => "(short) 0".to_owned(),
        JavaType::Int => "0".to_owned(),
        JavaType::Long => "0L".to_owned(),
        JavaType::Float => "0.0f".to_owned(),
        JavaType::Double => "0.0d".to_owned(),
        JavaType::Object(_) | JavaType::Array(_) => {
            format!("({}) null", names.value_type(value))
        }
    }
}

fn same_type(left: &str, right: &str) -> bool {
    left.bytes()
        .map(|byte| if byte == b'.' { b'/' } else { byte })
        .eq(right
            .bytes()
            .map(|byte| if byte == b'.' { b'/' } else { byte }))
}
