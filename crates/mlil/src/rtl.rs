//! Shared Java-managed semantics at the RTL/MLIL boundary.
//!
//! JVM and Dalvik keep distinct RTL dialect markers, storage policies,
//! edges, and low-level encoders. This module only supplies the semantic
//! constraint domain and the operation translation both targets use when
//! raising into, or lowering from, canonical Java MLIL.

use std::collections::{BTreeMap, BTreeSet};

use cfglib::ir::dialect::Vocabulary;
use cfglib::ir::rtl::{
    Constraint, Dialect as RtlDialect, Emission, Expr, Lift, LiftedStatement, Lower, LowerContext,
    MlilBridge, Shape, Statement, VarExpr,
};

use crate::{
    EdgeMetadata, EdgeRole, Effect, Instruction, JavaDialect, Operation, ValueType, VariableRole,
};

/// Finite hierarchy answers needed while merging one RTL function's webs.
///
/// Frontends build this table from the reference descriptors present in a
/// function. It keeps cfglib's constraint context concrete and borrow-free
/// while still accepting hierarchy adapters that borrow a classpath.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceMergeContext {
    common: BTreeMap<(String, String), Option<String>>,
}

impl ReferenceMergeContext {
    /// Queries every unordered pair of descriptors and records its common type.
    pub fn from_descriptors(
        descriptors: impl IntoIterator<Item = String>,
        mut common_supertype: impl FnMut(&str, &str) -> Option<String>,
    ) -> Self {
        let descriptors: Vec<_> = descriptors
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut common = BTreeMap::new();
        for (position, left) in descriptors.iter().enumerate() {
            for right in &descriptors[position + 1..] {
                common.insert((left.clone(), right.clone()), common_supertype(left, right));
            }
        }
        Self { common }
    }

    /// Returns the recorded common type of two descriptors.
    #[must_use]
    pub fn common_supertype(&self, left: &str, right: &str) -> Option<String> {
        if left == right {
            return Some(left.to_owned());
        }
        let key = if left < right {
            (left.to_owned(), right.to_owned())
        } else {
            (right.to_owned(), left.to_owned())
        };
        self.common.get(&key).cloned().flatten()
    }
}

/// Java-managed value constraint attached to one RTL storage value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManagedConstraint(pub ValueType);

impl ManagedConstraint {
    /// Wraps one canonical semantic value type.
    #[must_use]
    pub const fn new(value_type: ValueType) -> Self {
        Self(value_type)
    }

    /// Returns the canonical semantic value type.
    #[must_use]
    pub const fn value_type(&self) -> &ValueType {
        &self.0
    }
}

impl Constraint for ManagedConstraint {
    type Context = ();

    fn free() -> Self {
        Self(ValueType::Unknown)
    }

    fn conflicted() -> Self {
        Self(ValueType::Conflict)
    }

    fn merge(&self, other: &Self, (): &Self::Context) -> Option<Self> {
        merge_value_types(&self.0, &other.0, |_, _| None).map(Self)
    }

    fn width(&self) -> Option<u32> {
        value_width(&self.0)
    }
}

/// Constraint conversion required by the shared RTL operation adapter.
pub trait ValueConstraint: Constraint {
    /// Creates a target constraint from one canonical MLIL value type.
    fn from_value_type(value_type: ValueType) -> Self;

    /// Converts the resolved constraint into canonical MLIL typing.
    fn into_value_type(self) -> ValueType;
}

impl ValueConstraint for ManagedConstraint {
    fn from_value_type(value_type: ValueType) -> Self {
        Self(value_type)
    }

    fn into_value_type(self) -> ValueType {
        self.0
    }
}

/// Conservatively merges two Java-managed value types.
///
/// `common_supertype` is consulted only for two distinct exact reference
/// descriptors. Returning `None` falls back to `java/lang/Object`.
#[must_use]
pub fn merge_value_types(
    left: &ValueType,
    right: &ValueType,
    mut common_supertype: impl FnMut(&str, &str) -> Option<String>,
) -> Option<ValueType> {
    use ValueType as T;

    if left == right {
        return Some(left.clone());
    }
    match (left, right) {
        (T::Unknown, value) | (value, T::Unknown) => value.clone(),
        (T::Zero, value @ (T::Boolean | T::Integer | T::Float | T::Null | T::Reference(_)))
        | (value @ (T::Boolean | T::Integer | T::Float | T::Null | T::Reference(_)), T::Zero) => {
            value.clone()
        }
        (T::Null, T::Reference(descriptor)) | (T::Reference(descriptor), T::Null) => {
            T::Reference(descriptor.clone())
        }
        (T::Reference(None), T::Reference(_)) | (T::Reference(_), T::Reference(None)) => {
            T::Reference(None)
        }
        (T::Reference(Some(left)), T::Reference(Some(right))) => T::Reference(
            common_supertype(left, right).or_else(|| Some("Ljava/lang/Object;".to_owned())),
        ),
        (T::Bits32, T::Boolean | T::Integer | T::Float | T::Zero)
        | (T::Boolean | T::Integer | T::Float | T::Zero, T::Bits32) => T::Bits32,
        (T::Bits64, T::Long | T::Double) | (T::Long | T::Double, T::Bits64) => T::Bits64,
        _ => return None,
    }
    .into()
}

/// Returns the fixed bit width of one semantic value when known.
#[must_use]
pub const fn value_width(value_type: &ValueType) -> Option<u32> {
    match value_type {
        ValueType::Long | ValueType::Double | ValueType::Bits64 => Some(64),
        ValueType::Unknown | ValueType::Conflict => None,
        ValueType::Boolean
        | ValueType::Integer
        | ValueType::Float
        | ValueType::Bits32
        | ValueType::Zero
        | ValueType::Null
        | ValueType::Reference(_)
        | ValueType::UninitializedThis(_)
        | ValueType::Uninitialized { .. }
        | ValueType::ReturnAddress => Some(32),
    }
}

/// Emits one target RTL statement as canonical Java MLIL.
///
/// # Errors
///
/// Returns an error for an expression form that cannot denote one flat
/// semantic operation or for invalid cfglib emission.
pub fn emit<R>(
    context: &mut Emission<'_, '_, R>,
    statement: LiftedStatement<R>,
) -> cfglib::ir::rtl::Result<()>
where
    R: Lift<Operator = Operation, EffectOp = Operation> + MlilBridge<Mlil = JavaDialect>,
    R: Vocabulary<ValueType = ValueType, Effect = Effect, VariableRole = VariableRole>,
    R::Constraint: ValueConstraint,
{
    match statement {
        LiftedStatement::Assign { value, .. } => {
            let operation = value_operation(&value)?;
            if context.has_exceptional_successors() {
                let target = context
                    .target()
                    .cloned()
                    .ok_or_else(|| rtl_error("throwing assignment has no target"))?;
                let temporary =
                    context.temporary(VariableRole::Temporary, target.value_type.clone())?;
                context.append(
                    operation,
                    context.reads().to_vec(),
                    vec![temporary.clone()],
                    true,
                )?;
                context.continuation(EdgeMetadata::ordinary(EdgeRole::Commit))?;
                context.append(Operation::Copy, vec![temporary], vec![target], false)?;
            } else {
                context.single(operation)?;
            }
        }
        LiftedStatement::Effect { operation, .. } | LiftedStatement::Raise { operation, .. } => {
            context.single(operation)?;
        }
        LiftedStatement::Branch { condition } => {
            context.single(value_operation(&condition)?)?;
        }
        LiftedStatement::Dispatch { scrutinee } => {
            context.single(value_operation(&scrutinee)?)?;
        }
        LiftedStatement::Return { .. } => {
            context.single(Operation::Return)?;
        }
    }
    Ok(())
}

/// Lowers one canonical Java MLIL instruction into target RTL storage.
///
/// # Errors
///
/// Returns an error for invalid placement or a semantic instruction whose
/// result arity cannot be represented as register transfers.
pub fn lower_instruction<R>(
    context: &mut LowerContext<'_, R>,
    instruction: &Instruction,
) -> cfglib::ir::rtl::Result<()>
where
    R: Lower<Operator = Operation, EffectOp = Operation> + MlilBridge<Mlil = JavaDialect>,
    R: Vocabulary<ValueType = ValueType, Effect = Effect, VariableRole = VariableRole>,
    R::Constraint: ValueConstraint,
{
    let operands = instruction
        .uses()
        .iter()
        .zip(instruction.use_types())
        .map(|(&variable, value_type)| {
            context.read(variable, R::Constraint::from_value_type(value_type.clone()))
        })
        .collect::<cfglib::ir::rtl::Result<Vec<_>>>()?;
    let definitions = instruction
        .defs()
        .iter()
        .zip(instruction.def_types())
        .map(|(&variable, value_type)| Ok((context.place(variable)?.clone(), value_type.clone())))
        .collect::<cfglib::ir::rtl::Result<Vec<_>>>()?;
    context.emit(lower_operation::<R>(
        instruction.operation().clone(),
        operands,
        definitions,
        instruction.effects().to_vec(),
        instruction.may_throw(),
    )?)?;
    Ok(())
}

/// Converts one typed semantic operation into a target RTL statement.
///
/// This is shared by MLIL lowering and native LLIL semantic decoders, so
/// both paths select the same control, transfer, and effect forms.
///
/// # Errors
///
/// Returns an error for mismatched parallel-result arity or an operation
/// with multiple results that has no lossless RTL transfer form.
pub fn lower_operation<R>(
    operation: Operation,
    operands: Vec<Expr<R>>,
    definitions: Vec<(cfglib::ir::rtl::Place<R>, ValueType)>,
    effects: Vec<Effect>,
    may_throw: bool,
) -> cfglib::ir::rtl::Result<Statement<R>>
where
    R: RtlDialect<Operator = Operation, EffectOp = Operation> + Vocabulary<Effect = Effect>,
    R::Constraint: ValueConstraint,
{
    Ok(match &operation {
        Operation::Branch(_) => Statement::Branch {
            condition: application(operation, operands, ValueType::Boolean),
        },
        Operation::Switch(_) => Statement::Dispatch {
            scrutinee: application(operation, operands, ValueType::Integer),
        },
        Operation::Return => Statement::Return { values: operands },
        Operation::Throw => Statement::Raise {
            operation,
            operands,
            effects,
        },
        Operation::Copy | Operation::ParallelCopy | Operation::TypeRefine
            if definitions.len() > 1 =>
        {
            if operands.len() != definitions.len() {
                return Err(rtl_error("parallel copy operand/result arity differs"));
            }
            let assignments = definitions
                .into_iter()
                .zip(operands)
                .map(|((place, value_type), operand)| {
                    let value = if matches!(operation, Operation::TypeRefine) {
                        Expr::Reinterpret {
                            operand: Box::new(operand),
                            shape: Shape::scalar(R::Constraint::from_value_type(value_type)),
                        }
                    } else {
                        operand
                    };
                    (place, value)
                })
                .collect();
            Statement::Transfer {
                assignments,
                effects,
                may_throw,
            }
        }
        _ if definitions.is_empty() => Statement::Effect {
            operation,
            operands,
            effects,
            may_throw,
        },
        _ if definitions.len() == 1 => {
            let (target, value_type) = definitions
                .into_iter()
                .next()
                .ok_or_else(|| rtl_error("single-result operation has no result"))?;
            let value = match operation {
                Operation::Copy if operands.len() == 1 => operands
                    .into_iter()
                    .next()
                    .ok_or_else(|| rtl_error("copy has no operand"))?,
                Operation::TypeRefine if operands.len() == 1 => Expr::Reinterpret {
                    operand: Box::new(
                        operands
                            .into_iter()
                            .next()
                            .ok_or_else(|| rtl_error("type refinement has no operand"))?,
                    ),
                    shape: Shape::scalar(R::Constraint::from_value_type(value_type)),
                },
                operation => application(operation, operands, value_type),
            };
            Statement::Transfer {
                assignments: vec![(target, value)],
                effects,
                may_throw,
            }
        }
        _ => return Err(rtl_error("operation has unsupported multiple results")),
    })
}

fn application<R: RtlDialect<Operator = Operation>>(
    operator: Operation,
    operands: Vec<Expr<R>>,
    value_type: ValueType,
) -> Expr<R>
where
    R::Constraint: ValueConstraint,
{
    Expr::Apply {
        operator,
        operands,
        shape: Shape::scalar(R::Constraint::from_value_type(value_type)),
    }
}

fn value_operation<R>(value: &VarExpr<R>) -> cfglib::ir::rtl::Result<Operation>
where
    R: RtlDialect<Operator = Operation>,
{
    match value {
        VarExpr::Read { .. } => Ok(Operation::Copy),
        VarExpr::Apply { operator, .. } => Ok(operator.clone()),
        VarExpr::Reinterpret { .. } => Ok(Operation::TypeRefine),
        VarExpr::Const { .. } => Err(rtl_error(
            "raw RTL constants require a semantic constant operator",
        )),
        VarExpr::Compose { .. } => Err(rtl_error(
            "composed RTL lanes require target-specific legalization",
        )),
    }
}

fn rtl_error(message: impl Into<String>) -> cfglib::ir::rtl::Error {
    cfglib::ir::rtl::Error::Lowering(message.into())
}
