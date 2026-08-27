//! Dalvik specialization of cfglib RTL contracts.

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

/// Dalvik RTL dialect marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DexRtlDialect;

/// Storage locations used by Dalvik RTL before semantic variable recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DexStorage {
    /// A register retained from the source DEX code item.
    SourceRegister(u16),
    /// The source DEX implicit invocation or filled-array result channel.
    SourceResult,
    /// The source DEX implicit delivered-exception channel.
    SourceException,
    /// A register allocated while targeting Dalvik RTL from another source ISA.
    GeneratedRegister(u16),
    /// A synthetic value used only while expanding one semantic operation.
    Temporary(u32),
}

/// Dalvik register-analysis-aware RTL web constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegisterConstraint(ValueType);

impl RegisterConstraint {
    pub(crate) const fn value_type(&self) -> &ValueType {
        &self.0
    }
}

impl Constraint for RegisterConstraint {
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

impl mlil::rtl::ValueConstraint for RegisterConstraint {
    fn from_value_type(value_type: ValueType) -> Self {
        Self(value_type)
    }

    fn into_value_type(self) -> ValueType {
        self.0
    }
}

/// Exact Dalvik RTL edge metadata before MLIL instruction identities exist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EdgeMetadata {
    /// Semantic control-flow role, including native catch order/range.
    pub role: EdgeRole,
    /// RTL statement that may transfer through this exception edge.
    pub throw_site: Option<StatementId>,
}

impl EdgeMetadata {
    /// Creates ordinary Dalvik RTL edge metadata.
    #[must_use]
    pub const fn ordinary(role: EdgeRole) -> Self {
        Self {
            role,
            throw_site: None,
        }
    }
}

/// One checked Dalvik RTL function.
pub type Function = rtl::Function<DexRtlDialect>;

/// Result and rewrite maps from lowering canonical MLIL into Dalvik RTL.
pub type Lowered = rtl::Lowered<DexRtlDialect>;

impl Vocabulary for DexRtlDialect {
    type ValueType = ValueType;
    type Effect = Effect;
    type Source = FunctionCoordinate;
    type SourceSpan = AddressRange;
    type SourcePoint = CodeAddress;
    type VariableRole = VariableRole;
    type NativeVariable = DexStorage;

    fn span_is_empty(span: &Self::SourceSpan) -> bool {
        span.is_empty()
    }

    fn span_contains(span: &Self::SourceSpan, point: &Self::SourcePoint) -> bool {
        span.contains(*point)
    }
}

impl Dialect for DexRtlDialect {
    type Constraint = RegisterConstraint;
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

impl MlilBridge for DexRtlDialect {
    type Mlil = JavaDialect;
}

impl Lift for DexRtlDialect {
    fn value_type(shape: Shape<Self::Constraint>) -> ValueType {
        mlil::rtl::ValueConstraint::into_value_type(shape.scalar)
    }

    fn web_role(storage: Option<&DexStorage>) -> VariableRole {
        match storage {
            Some(DexStorage::SourceResult | DexStorage::Temporary(_)) | None => {
                VariableRole::Temporary
            }
            Some(DexStorage::SourceException) => VariableRole::Exception,
            Some(DexStorage::SourceRegister(_) | DexStorage::GeneratedRegister(_)) => {
                VariableRole::Local
            }
        }
    }

    fn parameter_role(ordinal: u16, _storage: &DexStorage) -> VariableRole {
        VariableRole::Parameter(ordinal)
    }

    fn native_variable(
        storage: &DexStorage,
        _source: &FunctionCoordinate,
    ) -> Option<NativeVariable> {
        let storage = match *storage {
            DexStorage::SourceRegister(index) => SourceStorage::DexRegister(index),
            DexStorage::SourceResult => SourceStorage::DexResult,
            DexStorage::SourceException => SourceStorage::DexException,
            DexStorage::GeneratedRegister(_) | DexStorage::Temporary(_) => return None,
        };
        Some(NativeVariable {
            format: BinaryFormat::Dex,
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
                        "Dalvik exception edge names a statement without an emitted throw site"
                            .into(),
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

impl Lower for DexRtlDialect {
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
                            "Dalvik exception edge names an instruction without a lowered statement"
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
