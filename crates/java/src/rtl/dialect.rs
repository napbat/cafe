//! JVM specialization of cfglib RTL contracts.

use cfglib::EdgeKind;
use cfglib::ir::dialect::Vocabulary;
use cfglib::ir::rtl::{
    self, Constraint, Dialect, EdgeContext, Lift, Lower, LowerContext, LowerEdgeContext,
    MlilBridge, Shape, StatementId,
};
use disassembler::{AddressRange, BinaryFormat, CodeAddress, FunctionCoordinate};
use mlil::{
    EdgeMetadata as MlilEdge, EdgeRole, Effect, JavaDialect, NativeVariable, Operation,
    SourceStorage, ValueType, VariableRole,
};

/// JVM RTL dialect marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JvmRtlDialect;

/// Storage locations used by JVM RTL before semantic variable recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JvmStorage {
    /// A local slot retained from the source class file.
    SourceLocal(u16),
    /// An operand-stack position retained from the source class file.
    SourceStack(u16),
    /// A local allocated while targeting JVM RTL from another source ISA.
    GeneratedLocal(u16),
    /// A synthetic value used only while expanding one semantic operation.
    Temporary(u32),
}

/// JVM verifier-aware RTL web constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JvmConstraint(ValueType);

impl JvmConstraint {
    pub(crate) const fn value_type(&self) -> &ValueType {
        &self.0
    }
}

impl Constraint for JvmConstraint {
    type Context = mlil::rtl::ReferenceMergeContext;

    fn free() -> Self {
        Self(ValueType::Unknown)
    }

    fn conflicted() -> Self {
        Self(ValueType::Conflict)
    }

    fn merge(&self, other: &Self, hierarchy: &Self::Context) -> Option<Self> {
        mlil::rtl::merge_value_types(&self.0, &other.0, |left, right| {
            hierarchy.common_supertype(left, right)
        })
        .map(Self)
    }

    fn width(&self) -> Option<u32> {
        mlil::rtl::value_width(&self.0)
    }
}

impl mlil::rtl::ValueConstraint for JvmConstraint {
    fn from_value_type(value_type: ValueType) -> Self {
        Self(value_type)
    }

    fn into_value_type(self) -> ValueType {
        self.0
    }
}

/// Exact JVM RTL edge metadata before MLIL instruction identities exist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EdgeMetadata {
    /// Semantic control-flow role, including native catch order/range.
    pub role: EdgeRole,
    /// RTL statement that may transfer through this exception edge.
    pub throw_site: Option<StatementId>,
}

impl EdgeMetadata {
    /// Creates ordinary JVM RTL edge metadata.
    #[must_use]
    pub const fn ordinary(role: EdgeRole) -> Self {
        Self {
            role,
            throw_site: None,
        }
    }
}

/// One checked JVM RTL function.
pub type Function = rtl::Function<JvmRtlDialect>;

/// Result and rewrite maps from lowering canonical MLIL into JVM RTL.
pub type Lowered = rtl::Lowered<JvmRtlDialect>;

impl Vocabulary for JvmRtlDialect {
    type ValueType = ValueType;
    type Effect = Effect;
    type Source = FunctionCoordinate;
    type SourceSpan = AddressRange;
    type SourcePoint = CodeAddress;
    type VariableRole = VariableRole;
    type NativeVariable = JvmStorage;

    fn span_is_empty(span: &Self::SourceSpan) -> bool {
        span.is_empty()
    }

    fn span_contains(span: &Self::SourceSpan, point: &Self::SourcePoint) -> bool {
        span.contains(*point)
    }
}

impl Dialect for JvmRtlDialect {
    type Constraint = JvmConstraint;
    type Operator = Operation;
    type EffectOp = Operation;
    type Edge = EdgeMetadata;

    fn mnemonic(operator: &Self::Operator) -> &str {
        operator.mnemonic()
    }

    fn effect_mnemonic(operation: &Self::EffectOp) -> &str {
        operation.mnemonic()
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        edge_kind(&edge.role)
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        edge.role == EdgeRole::Entry
    }
}

impl MlilBridge for JvmRtlDialect {
    type Mlil = JavaDialect;
}

impl Lift for JvmRtlDialect {
    fn value_type(shape: Shape<Self::Constraint>) -> ValueType {
        mlil::rtl::ValueConstraint::into_value_type(shape.scalar)
    }

    fn web_role(storage: Option<&JvmStorage>) -> VariableRole {
        match storage {
            Some(JvmStorage::SourceLocal(_) | JvmStorage::GeneratedLocal(_)) => VariableRole::Local,
            Some(JvmStorage::SourceStack(_) | JvmStorage::Temporary(_)) | None => {
                VariableRole::Temporary
            }
        }
    }

    fn parameter_role(ordinal: u16, _storage: &JvmStorage) -> VariableRole {
        VariableRole::Parameter(ordinal)
    }

    fn native_variable(
        storage: &JvmStorage,
        _source: &FunctionCoordinate,
    ) -> Option<NativeVariable> {
        let storage = match *storage {
            JvmStorage::SourceLocal(index) => SourceStorage::JvmLocal(index),
            JvmStorage::SourceStack(index) => SourceStorage::JvmStack(index),
            JvmStorage::GeneratedLocal(_) | JvmStorage::Temporary(_) => return None,
        };
        Some(NativeVariable {
            format: BinaryFormat::JavaClass,
            storage,
        })
    }

    fn emit(
        context: &mut rtl::Emission<'_, '_, Self>,
        statement: rtl::LiftedStatement<Self>,
    ) -> rtl::Result<()> {
        mlil::rtl::emit(context, statement)
    }

    fn lift_edge(edge: &EdgeMetadata, context: &EdgeContext<'_>) -> rtl::Result<MlilEdge> {
        let throw_site = edge
            .throw_site
            .or_else(|| edge.role.is_exception().then(|| context.owner()).flatten())
            .map(|statement| {
                context.throw_site(statement).ok_or_else(|| {
                    rtl::Error::Lifting(
                        "JVM exception edge names a statement without an emitted throw site".into(),
                    )
                })
            })
            .transpose()?;
        Ok(MlilEdge {
            role: edge.role.clone(),
            throw_site,
        })
    }
}

impl Lower for JvmRtlDialect {
    fn plan(function: &mlil::Function) -> rtl::Result<rtl::Placement<Self>> {
        super::placement::plan(function)
    }

    fn lower_instruction(
        context: &mut LowerContext<'_, Self>,
        instruction: &mlil::Instruction,
    ) -> rtl::Result<()> {
        mlil::rtl::lower_instruction(context, instruction)
    }

    fn lower_edge(edge: &MlilEdge, context: &LowerEdgeContext<'_>) -> rtl::Result<EdgeMetadata> {
        let throw_site = edge
            .throw_site
            .or_else(|| edge.role.is_exception().then(|| context.owner()).flatten())
            .map(|instruction| {
                context
                    .statements(instruction)
                    .first()
                    .copied()
                    .ok_or_else(|| {
                        rtl::Error::Lowering(
                            "JVM exception edge names an instruction without a lowered statement"
                                .into(),
                        )
                    })
            })
            .transpose()?;
        Ok(EdgeMetadata {
            role: edge.role.clone(),
            throw_site,
        })
    }
}

fn edge_kind(role: &EdgeRole) -> EdgeKind {
    match role {
        EdgeRole::Entry | EdgeRole::Commit | EdgeRole::FallThrough => EdgeKind::Fallthrough,
        EdgeRole::BranchTrue => EdgeKind::ConditionalTrue,
        EdgeRole::BranchFalse => EdgeKind::ConditionalFalse,
        EdgeRole::Jump => EdgeKind::Jump,
        EdgeRole::SwitchDefault => EdgeKind::Unconditional,
        EdgeRole::SwitchCase(_) => EdgeKind::SwitchCase,
        EdgeRole::Exception { .. } => EdgeKind::ExceptionUnwind,
    }
}
