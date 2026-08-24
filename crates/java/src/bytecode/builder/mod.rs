//! Label-based construction and automatic layout of JVM method bytecode.

mod layout;
mod model;

use model::{
    BranchForm, PendingExceptionHandler, PendingInstruction, PendingInstructionKind,
    next_builder_scope,
};

use super::{Opcode, Operand};
use crate::{Error, Result};

pub use self::model::{BuiltCode, CatchTarget, InstructionId, Label, LocalKind};

/// Symbolic JVM bytecode builder with automatic offsets and branch relaxation.
///
/// Instructions receive stable identities as they are requested. Labels name
/// item boundaries and remain valid when layout changes instruction widths.
#[derive(Debug)]
pub struct CodeBuilder {
    scope: u64,
    labels: Vec<Option<usize>>,
    instructions: Vec<PendingInstruction>,
    handlers: Vec<PendingExceptionHandler>,
}

impl CodeBuilder {
    /// Creates an empty symbolic method body.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scope: next_builder_scope(),
            labels: Vec::new(),
            instructions: Vec::new(),
            handlers: Vec::new(),
        }
    }

    /// Allocates an unbound label owned by this builder.
    #[must_use]
    pub fn new_label(&mut self) -> Label {
        let label = Label {
            scope: self.scope,
            index: self.labels.len(),
        };
        self.labels.push(None);
        label
    }

    /// Binds a label to the boundary before the next requested instruction.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign or already-bound label.
    pub fn bind(&mut self, label: Label) -> Result<()> {
        self.require_label(label)?;
        let position = self.instructions.len();
        let binding = &mut self.labels[label.index];
        if let Some(existing) = binding {
            return Err(Error::invalid_assembly(format!(
                "symbolic bytecode label {} is already bound at item {existing}",
                label.index
            )));
        }
        *binding = Some(position);
        Ok(())
    }

    /// Requests an instruction without symbolic control-flow operands.
    ///
    /// Local-variable width and `ldc` versus `ldc_w` are inferred during
    /// layout. Use the dedicated branch and switch methods for control flow.
    #[must_use]
    pub fn emit(&mut self, opcode: Opcode, operand: Operand) -> InstructionId {
        self.push(PendingInstructionKind::Plain { opcode, operand })
    }

    /// Requests a direct branch to a symbolic target.
    ///
    /// Short `goto` and `jsr` instructions widen automatically. A distant
    /// conditional is inverted around a synthetic `goto_w`.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign target or a non-branch opcode.
    pub fn emit_branch(&mut self, opcode: Opcode, target: Label) -> Result<InstructionId> {
        self.require_label(target)?;
        if !opcode.is_conditional_branch() && !opcode.is_unconditional_branch() {
            return Err(Error::invalid_assembly(format!(
                "{} is not a direct branch opcode",
                opcode.mnemonic()
            )));
        }
        let form = if matches!(opcode, Opcode::GotoW | Opcode::JsrW) {
            BranchForm::Wide
        } else {
            BranchForm::Short
        };
        Ok(self.push(PendingInstructionKind::Branch {
            opcode,
            target,
            form,
        }))
    }

    /// Requests a dense integer switch with symbolic targets.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign target or an empty target table.
    pub fn emit_table_switch(
        &mut self,
        default: Label,
        low: i32,
        targets: impl IntoIterator<Item = Label>,
    ) -> Result<InstructionId> {
        self.require_label(default)?;
        let targets = targets.into_iter().collect::<Vec<_>>();
        if targets.is_empty() {
            return Err(Error::invalid_assembly(
                "tableswitch requires at least one case target",
            ));
        }
        for &target in &targets {
            self.require_label(target)?;
        }
        Ok(self.push(PendingInstructionKind::TableSwitch {
            default,
            low,
            targets,
        }))
    }

    /// Requests a sparse integer switch with symbolic targets.
    ///
    /// # Errors
    ///
    /// Returns an error for foreign targets or keys that are not strictly
    /// increasing.
    pub fn emit_lookup_switch(
        &mut self,
        default: Label,
        pairs: impl IntoIterator<Item = (i32, Label)>,
    ) -> Result<InstructionId> {
        self.require_label(default)?;
        let pairs = pairs.into_iter().collect::<Vec<_>>();
        for &(_, target) in &pairs {
            self.require_label(target)?;
        }
        if pairs.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(Error::invalid_assembly(
                "lookupswitch keys must be strictly increasing",
            ));
        }
        Ok(self.push(PendingInstructionKind::LookupSwitch { default, pairs }))
    }

    /// Adds an exception handler using symbolic protected-range boundaries.
    ///
    /// # Errors
    ///
    /// Returns an error when any label belongs to another builder.
    pub fn add_exception_handler(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        catch: CatchTarget,
    ) -> Result<()> {
        self.require_label(start)?;
        self.require_label(end)?;
        self.require_label(handler)?;
        if catch == CatchTarget::Class(0) {
            return Err(Error::invalid_assembly(
                "catch-all handlers must use CatchTarget::Any",
            ));
        }
        self.handlers.push(PendingExceptionHandler {
            start,
            end,
            handler,
            catch,
        });
        Ok(())
    }

    /// Emits the narrowest local-variable load for a computational category.
    #[must_use]
    pub fn emit_load(&mut self, kind: LocalKind, index: u16) -> InstructionId {
        let (generic, compact) = local_load_opcodes(kind);
        self.emit_local(generic, compact, index)
    }

    /// Emits the narrowest local-variable store for a computational category.
    #[must_use]
    pub fn emit_store(&mut self, kind: LocalKind, index: u16) -> InstructionId {
        let (generic, compact) = local_store_opcodes(kind);
        self.emit_local(generic, compact, index)
    }

    /// Emits `ldc` or `ldc_w` according to the constant-pool index width.
    #[must_use]
    pub fn emit_ldc(&mut self, index: u16) -> InstructionId {
        let opcode = if u8::try_from(index).is_ok() {
            Opcode::Ldc
        } else {
            Opcode::LdcW
        };
        self.emit(opcode, Operand::Constant(index))
    }

    /// Emits `ldc2_w` for a category-two numeric constant.
    #[must_use]
    pub fn emit_ldc2(&mut self, index: u16) -> InstructionId {
        self.emit(Opcode::Ldc2W, Operand::Constant(index))
    }

    /// Resolves labels, relaxes instruction forms, and assembles the method.
    ///
    /// # Errors
    ///
    /// Returns an error for unbound labels, invalid operand/opcode pairs,
    /// unrepresentable metadata, or a method exceeding the JVM code limit.
    pub fn finish(self) -> Result<BuiltCode> {
        let Self {
            scope,
            labels,
            instructions,
            handlers,
        } = self;
        layout::finish(scope, instructions, &labels, &handlers)
    }

    fn emit_local(&mut self, generic: Opcode, compact: [Opcode; 4], index: u16) -> InstructionId {
        let position = usize::from(index);
        if position < compact.len() {
            self.emit(compact[position], Operand::None)
        } else {
            self.emit(generic, Operand::Local(index))
        }
    }

    fn push(&mut self, kind: PendingInstructionKind) -> InstructionId {
        let id = InstructionId {
            scope: self.scope,
            index: self.instructions.len(),
        };
        self.instructions.push(PendingInstruction { kind });
        id
    }

    fn require_label(&self, label: Label) -> Result<()> {
        if label.scope != self.scope || label.index >= self.labels.len() {
            Err(Error::invalid_assembly(
                "symbolic bytecode label belongs to another builder",
            ))
        } else {
            Ok(())
        }
    }
}

impl Default for CodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn local_load_opcodes(kind: LocalKind) -> (Opcode, [Opcode; 4]) {
    match kind {
        LocalKind::Integer => (
            Opcode::ILoad,
            [
                Opcode::ILoad0,
                Opcode::ILoad1,
                Opcode::ILoad2,
                Opcode::ILoad3,
            ],
        ),
        LocalKind::Long => (
            Opcode::LLoad,
            [
                Opcode::LLoad0,
                Opcode::LLoad1,
                Opcode::LLoad2,
                Opcode::LLoad3,
            ],
        ),
        LocalKind::Float => (
            Opcode::FLoad,
            [
                Opcode::FLoad0,
                Opcode::FLoad1,
                Opcode::FLoad2,
                Opcode::FLoad3,
            ],
        ),
        LocalKind::Double => (
            Opcode::DLoad,
            [
                Opcode::DLoad0,
                Opcode::DLoad1,
                Opcode::DLoad2,
                Opcode::DLoad3,
            ],
        ),
        LocalKind::Reference => (
            Opcode::ALoad,
            [
                Opcode::ALoad0,
                Opcode::ALoad1,
                Opcode::ALoad2,
                Opcode::ALoad3,
            ],
        ),
    }
}

fn local_store_opcodes(kind: LocalKind) -> (Opcode, [Opcode; 4]) {
    match kind {
        LocalKind::Integer => (
            Opcode::IStore,
            [
                Opcode::IStore0,
                Opcode::IStore1,
                Opcode::IStore2,
                Opcode::IStore3,
            ],
        ),
        LocalKind::Long => (
            Opcode::LStore,
            [
                Opcode::LStore0,
                Opcode::LStore1,
                Opcode::LStore2,
                Opcode::LStore3,
            ],
        ),
        LocalKind::Float => (
            Opcode::FStore,
            [
                Opcode::FStore0,
                Opcode::FStore1,
                Opcode::FStore2,
                Opcode::FStore3,
            ],
        ),
        LocalKind::Double => (
            Opcode::DStore,
            [
                Opcode::DStore0,
                Opcode::DStore1,
                Opcode::DStore2,
                Opcode::DStore3,
            ],
        ),
        LocalKind::Reference => (
            Opcode::AStore,
            [
                Opcode::AStore0,
                Opcode::AStore1,
                Opcode::AStore2,
                Opcode::AStore3,
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{CatchTarget, CodeBuilder, LocalKind};
    use crate::bytecode::{Opcode, Operand};
    use crate::classfile::{CodeAttribute, ConstantPool};

    #[test]
    fn resolves_labels_compact_locals_and_exception_ranges() {
        let mut builder = CodeBuilder::new();
        let start = builder.new_label();
        let end = builder.new_label();
        let handler = builder.new_label();
        builder.bind(start).unwrap();
        let load = builder.emit_load(LocalKind::Reference, 0);
        let _ = builder.emit(Opcode::Pop, Operand::None);
        builder.bind(end).unwrap();
        let _ = builder.emit(Opcode::Return, Operand::None);
        builder.bind(handler).unwrap();
        let _ = builder.emit(Opcode::AThrow, Operand::None);
        builder
            .add_exception_handler(start, end, handler, CatchTarget::Any)
            .unwrap();

        let built = builder.finish().unwrap();
        assert_eq!(built.instruction_offset(load), Some(0));
        assert_eq!(built.instructions()[0].opcode, Opcode::ALoad0);
        assert_eq!(built.exception_table().len(), 1);
        assert_eq!(built.exception_table()[0].start_pc, 0);
        assert_eq!(built.exception_table()[0].end_pc, 2);
        let generated = built.label_range(start, end).unwrap();
        assert_eq!(generated.start.get(), 0);
        assert_eq!(generated.end.get(), 2);
        let mut pool = ConstantPool::new();
        let attribute = CodeAttribute::from_built(&mut pool, 1, 1, &built).unwrap();
        assert_eq!(attribute.code, built.code());
        assert_eq!(attribute.exception_table, built.exception_table());
    }

    #[test]
    fn selects_wide_local_and_constant_forms() {
        let mut builder = CodeBuilder::new();
        let _ = builder.emit_load(LocalKind::Integer, 300);
        let _ = builder.emit_ldc(300);
        let _ = builder.emit(Opcode::Return, Operand::None);
        let built = builder.finish().unwrap();
        assert!(built.instructions()[0].wide);
        assert_eq!(built.instructions()[1].opcode, Opcode::LdcW);
    }

    #[test]
    fn expands_distant_conditionals() {
        let mut builder = CodeBuilder::new();
        let target = builder.new_label();
        let conditional = builder.emit_branch(Opcode::IfEq, target).unwrap();
        for _ in 0..40_000 {
            let _ = builder.emit(Opcode::Nop, Operand::None);
        }
        builder.bind(target).unwrap();
        let _ = builder.emit(Opcode::Return, Operand::None);

        let built = builder.finish().unwrap();
        let offset = built.instruction_offset(conditional).unwrap();
        assert_eq!(offset, 0);
        assert_eq!(built.instructions()[0].opcode, Opcode::IfNe);
        assert_eq!(built.instructions()[1].opcode, Opcode::GotoW);
    }

    #[test]
    fn widens_distant_gotos() {
        let mut builder = CodeBuilder::new();
        let target = builder.new_label();
        let branch = builder.emit_branch(Opcode::Goto, target).unwrap();
        for _ in 0..40_000 {
            let _ = builder.emit(Opcode::Nop, Operand::None);
        }
        builder.bind(target).unwrap();
        let _ = builder.emit(Opcode::Return, Operand::None);

        let built = builder.finish().unwrap();
        let position = built.instruction_offset(branch).unwrap();
        let instruction = built
            .instructions()
            .iter()
            .find(|instruction| instruction.offset == position)
            .unwrap();
        assert_eq!(instruction.opcode, Opcode::GotoW);
    }

    #[test]
    fn aligns_symbolic_switches_and_resolves_targets() {
        let mut builder = CodeBuilder::new();
        let case = builder.new_label();
        let fallback = builder.new_label();
        let switch = builder.emit_table_switch(fallback, 7, [case]).unwrap();
        builder.bind(case).unwrap();
        let _ = builder.emit(Opcode::Return, Operand::None);
        builder.bind(fallback).unwrap();
        let _ = builder.emit(Opcode::Return, Operand::None);

        let built = builder.finish().unwrap();
        let instruction = &built.instructions()[0];
        assert_eq!(built.instruction_offset(switch), Some(0));
        assert_eq!(instruction.size, 20);
        let Operand::TableSwitch {
            default, targets, ..
        } = &instruction.operand
        else {
            panic!("expected a table switch");
        };
        assert_eq!(
            usize::try_from(*default).unwrap(),
            built.label_offset(fallback).unwrap()
        );
        assert_eq!(
            usize::try_from(targets[0]).unwrap(),
            built.label_offset(case).unwrap()
        );
    }

    #[test]
    fn rejects_foreign_and_unbound_labels() {
        let mut first = CodeBuilder::new();
        let unbound = first.new_label();
        let mut second = CodeBuilder::new();
        assert!(second.emit_branch(Opcode::Goto, unbound).is_err());
        let _ = first.emit(Opcode::Return, Operand::None);
        assert!(first.finish().is_err());
    }

    #[test]
    fn rejects_empty_and_oversized_method_bodies() {
        assert!(CodeBuilder::new().finish().is_err());

        let mut builder = CodeBuilder::new();
        for _ in 0..=crate::classfile::MAX_CODE_LENGTH {
            let _ = builder.emit(Opcode::Nop, Operand::None);
        }
        assert!(builder.finish().is_err());
    }
}
