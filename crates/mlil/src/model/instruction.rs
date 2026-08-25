//! Typed MLIL instruction storage and cfglib adapters.

use std::borrow::Cow;

use disassembler::cfglib::{
    CallInfo, DisplayInstr, EffectInfo, FlowControl, FlowEffect, InstrInfo,
};

use super::{ControlClass, Operation, TypedVariable, ValueType, VariableId};

/// Stable identity of one MLIL instruction within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstructionId(u32);

impl InstructionId {
    /// Creates an identity from its dense raw index.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the compact raw identity.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for InstructionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "i{}", self.0)
    }
}

/// Observable semantic effect beyond variable definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Effect {
    /// Read managed heap or array state.
    ReadMemory,
    /// Mutate managed heap or array state.
    WriteMemory,
    /// Allocate managed storage.
    Allocate,
    /// Invoke another method or dynamic call site.
    Call,
    /// Acquire or release synchronization state.
    Synchronize,
    /// May transfer through an exception edge or terminate exceptionally.
    Throw,
    /// Changes intraprocedural control flow or exits the function.
    Control,
}

/// One typed MLIL instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    id: InstructionId,
    operation: Operation,
    uses: Vec<VariableId>,
    use_types: Vec<ValueType>,
    defs: Vec<VariableId>,
    def_types: Vec<ValueType>,
    effects: Vec<Effect>,
}

impl Instruction {
    pub(crate) fn new(
        id: InstructionId,
        operation: Operation,
        uses: Vec<TypedVariable>,
        defs: Vec<TypedVariable>,
        may_throw: bool,
    ) -> Self {
        let (uses, use_types) = split_typed(uses);
        let (defs, def_types) = split_typed(defs);
        let effects = operation_effects(&operation, may_throw);
        Self {
            id,
            operation,
            uses,
            use_types,
            defs,
            def_types,
            effects,
        }
    }

    /// Returns the stable instruction identity.
    #[must_use]
    pub const fn id(&self) -> InstructionId {
        self.id
    }

    /// Returns the encoding-independent semantic operation.
    #[must_use]
    pub const fn operation(&self) -> &Operation {
        &self.operation
    }

    /// Returns variable uses in semantic operand order.
    #[must_use]
    pub fn uses(&self) -> &[VariableId] {
        &self.uses
    }

    /// Returns the type of every variable use in matching order.
    #[must_use]
    pub fn use_types(&self) -> &[ValueType] {
        &self.use_types
    }

    /// Returns variable definitions in semantic result order.
    #[must_use]
    pub fn defs(&self) -> &[VariableId] {
        &self.defs
    }

    /// Returns the type of every variable definition in matching order.
    #[must_use]
    pub fn def_types(&self) -> &[ValueType] {
        &self.def_types
    }

    /// Returns sorted, deduplicated observable effects.
    #[must_use]
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// Returns whether execution may transfer through an exception edge.
    #[must_use]
    pub fn may_throw(&self) -> bool {
        self.effects.contains(&Effect::Throw)
    }

    pub(crate) fn rewrite_use(&mut self, old: VariableId, new: VariableId) {
        for variable in &mut self.uses {
            if *variable == old {
                *variable = new;
            }
        }
    }
}

impl InstrInfo for Instruction {
    type Variable = VariableId;

    fn uses(&self) -> &[Self::Variable] {
        &self.uses
    }

    fn defs(&self) -> &[Self::Variable] {
        &self.defs
    }
}

impl EffectInfo for Instruction {
    type Effect = Effect;

    fn effects(&self) -> &[Self::Effect] {
        &self.effects
    }
}

impl FlowControl for Instruction {
    fn flow_effect(&self) -> FlowEffect {
        match self.operation.control_class() {
            ControlClass::Normal if self.may_throw() => FlowEffect::MayThrow,
            ControlClass::Normal => FlowEffect::Fallthrough,
            ControlClass::Branch => FlowEffect::ConditionalJump,
            ControlClass::Jump => FlowEffect::Jump,
            ControlClass::Switch => FlowEffect::IndirectJump,
            ControlClass::Return => FlowEffect::Return,
            ControlClass::Throw => FlowEffect::Terminate,
        }
    }
}

impl CallInfo for Instruction {
    type Callee = String;

    fn callee(&self) -> Option<Self::Callee> {
        let Operation::Call { target, .. } = &self.operation else {
            return None;
        };
        Some(
            target
                .display
                .clone()
                .unwrap_or_else(|| format!("{:?}#{}", target.kind, target.index)),
        )
    }
}

impl std::fmt::Display for Instruction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.defs.is_empty() {
            for (position, variable) in self.defs.iter().enumerate() {
                if position != 0 {
                    formatter.write_str(", ")?;
                }
                variable.fmt(formatter)?;
            }
            formatter.write_str(" = ")?;
        }
        formatter.write_str(self.operation.mnemonic())?;
        for (position, variable) in self.uses.iter().enumerate() {
            if position == 0 {
                formatter.write_str(" ")?;
            } else {
                formatter.write_str(", ")?;
            }
            variable.fmt(formatter)?;
        }
        Ok(())
    }
}

impl DisplayInstr for Instruction {
    fn mnemonic(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }
}

fn split_typed(values: Vec<TypedVariable>) -> (Vec<VariableId>, Vec<ValueType>) {
    values
        .into_iter()
        .map(|value| (value.variable, value.value_type))
        .unzip()
}

fn operation_effects(operation: &Operation, may_throw: bool) -> Vec<Effect> {
    use super::{ArrayAccess, FieldAccess};

    let mut effects = match operation {
        Operation::Array {
            access: ArrayAccess::Get,
            ..
        }
        | Operation::ArrayLength
        | Operation::Field {
            access: FieldAccess::GetInstance | FieldAccess::GetStatic,
            ..
        } => vec![Effect::ReadMemory],
        Operation::Array {
            access: ArrayAccess::Put,
            ..
        }
        | Operation::InitializeArray { .. }
        | Operation::Field {
            access: FieldAccess::PutInstance | FieldAccess::PutStatic,
            ..
        } => vec![Effect::WriteMemory],
        Operation::Call { .. } => vec![Effect::Call, Effect::ReadMemory, Effect::WriteMemory],
        Operation::Allocate(_) => vec![Effect::Allocate],
        Operation::Monitor(_) => vec![Effect::Synchronize],
        Operation::Branch(_)
        | Operation::Jump
        | Operation::Switch(_)
        | Operation::Return
        | Operation::Throw => vec![Effect::Control],
        Operation::Intrinsic(_) => vec![Effect::ReadMemory, Effect::WriteMemory],
        Operation::Nop
        | Operation::Copy
        | Operation::ParallelCopy
        | Operation::Discard
        | Operation::TypeRefine
        | Operation::Constant(_)
        | Operation::Unary(_)
        | Operation::Binary(_)
        | Operation::Convert(_)
        | Operation::Compare(_)
        | Operation::CheckCast(_)
        | Operation::InstanceOf(_)
        | Operation::CaughtException(_) => Vec::new(),
    };
    if may_throw || matches!(operation, Operation::Throw) {
        effects.push(Effect::Throw);
    }
    effects.sort_unstable();
    effects.dedup();
    effects
}
