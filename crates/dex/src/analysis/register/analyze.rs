//! Worklist solver, method entry state, and register-lattice merging.

use std::collections::BTreeMap;

use disassembler::cfglib::{
    Direction, EdgeId, EdgeRef, NodeId, Reachable, ReachableEdgeProblem, RootedGraphView,
    TrySolveError, try_solve_edge_problem_from,
};

use crate::file::{AccessFlags, CodeItem, DexFile, EncodedMethod, PrototypeId};
use crate::{Error, Result};

use super::super::flow::build_control_flow;
use super::super::{
    BodyAnalysis, ControlFlow, DexHierarchy, FlowEdge, FlowEdgeKind, ReferenceHierarchy,
    analyze_body, resolve_instruction_references,
};
use super::model::{ReferenceType, RegisterAnalysis, RegisterFrame, RegisterType};
use super::transfer::{descriptor_type, transfer};

const CONSTRUCTOR_NAME: &str = "<init>";

pub(super) struct MethodContext<'a> {
    pub(super) file: &'a DexFile,
    pub(super) declaration: &'a EncodedMethod,
    pub(super) code: &'a CodeItem,
    pub(super) body: &'a BodyAnalysis,
    pub(super) prototype: &'a PrototypeId,
    pub(super) owner: &'a str,
    pub(super) name: &'a str,
    pub(super) hierarchy: &'a dyn ReferenceHierarchy,
}

impl MethodContext<'_> {
    pub(super) fn is_constructor(&self) -> bool {
        self.name == CONSTRUCTOR_NAME
    }
}

/// Computes register types for a method using its enclosing DEX hierarchy.
///
/// Abstract or native methods return `None` because they have no code item.
///
/// # Errors
///
/// Returns an error for invalid references, body structure, control flow, or a
/// reachable instruction that reads an incompatible register state.
pub fn analyze_method_registers(
    file: &DexFile,
    declaration: &EncodedMethod,
) -> Result<Option<RegisterAnalysis>> {
    if declaration.code.is_none() {
        return Ok(None);
    }
    let hierarchy = DexHierarchy::from_file(file)?;
    analyze_method_registers_with_hierarchy(file, declaration, &hierarchy)
}

/// Computes register types with a caller-supplied classpath hierarchy.
///
/// A custom hierarchy can provide relationships for external classes absent
/// from the current DEX file. Abstract or native methods return `None`.
///
/// # Errors
///
/// Returns an error for invalid references, body structure, control flow, or a
/// reachable instruction that reads an incompatible register state.
pub fn analyze_method_registers_with_hierarchy(
    file: &DexFile,
    declaration: &EncodedMethod,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<RegisterAnalysis>> {
    let Some(code) = &declaration.code else {
        return Ok(None);
    };
    let identity = file.resolve_method(declaration.method)?;
    let owner = identity.owner.to_owned();
    let name = identity.name.to_owned();
    let signature = identity.signature;
    analyze_inner(file, declaration, code, hierarchy, &owner, &name)
        .map(Some)
        .map_err(|error| error.in_method(owner, name, signature))
}

fn analyze_inner(
    file: &DexFile,
    declaration: &EncodedMethod,
    code: &CodeItem,
    hierarchy: &dyn ReferenceHierarchy,
    owner: &str,
    name: &str,
) -> Result<RegisterAnalysis> {
    let method = file.resolve_method_id(declaration.method)?;
    let prototype = file.resolve_prototype(method.prototype)?;
    let body = analyze_body(code)?;
    for instruction in &code.instructions {
        let _ = resolve_instruction_references(file, instruction)?;
    }
    let flow = build_control_flow(code, &body)?;
    let context = MethodContext {
        file,
        declaration,
        code,
        body: &body,
        prototype,
        owner,
        name,
        hierarchy,
    };
    let instructions = code
        .instructions
        .iter()
        .map(|instruction| (instruction.offset(), instruction))
        .collect::<BTreeMap<_, _>>();
    let entry = flow.root();
    let initial = initial_frame(&context)?;
    let problem = RegisterProblem {
        context,
        instructions,
        entry,
        initial,
    };
    let facts =
        try_solve_edge_problem_from(&flow, &Reachable(problem), &[entry]).map_err(|error| {
            match error {
                TrySolveError::Problem(error) => error,
                TrySolveError::Solver(error) => {
                    Error::invalid_instruction(flow.entry(), error.to_string())
                }
            }
        })?;

    let mut entries = BTreeMap::new();
    let mut exits = BTreeMap::new();
    for (node, &offset) in flow.nodes().iter().enumerate() {
        if let Some(frame) = facts.fact_in(node) {
            entries.insert(offset, frame.clone());
        }
        if let Some(frame) = facts.fact_out(node) {
            exits.insert(offset, frame.clone());
        }
    }

    Ok(RegisterAnalysis {
        flow,
        entries,
        exits,
    })
}

struct RegisterProblem<'method> {
    context: MethodContext<'method>,
    instructions: BTreeMap<u32, &'method crate::instruction::Instruction>,
    entry: NodeId,
    initial: RegisterFrame,
}

impl ReachableEdgeProblem<ControlFlow> for RegisterProblem<'_> {
    type Fact = RegisterFrame;
    type Error = Error;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn entry_fact(&self, _graph: &ControlFlow, node: NodeId) -> Result<Option<RegisterFrame>> {
        Ok((node == self.entry).then(|| self.initial.clone()))
    }

    fn merge(
        &self,
        _graph: &ControlFlow,
        _node: NodeId,
        left: &RegisterFrame,
        right: &RegisterFrame,
    ) -> Result<RegisterFrame> {
        let mut merged = left.clone();
        let _ = merge_frames(&mut merged, right, self.context.hierarchy);
        Ok(merged)
    }

    fn transfer(
        &self,
        graph: &ControlFlow,
        node: NodeId,
        input: &RegisterFrame,
    ) -> Result<RegisterFrame> {
        let offset = graph.node_offset(node);
        transfer(&self.context, self.instructions[&offset], input)
    }

    fn edge_observes_input(
        &self,
        _graph: &ControlFlow,
        edge: EdgeRef<'_, NodeId, EdgeId, FlowEdge>,
    ) -> bool {
        matches!(edge.data().kind, FlowEdgeKind::Exception(_))
    }
}

fn initial_frame(context: &MethodContext<'_>) -> Result<RegisterFrame> {
    let register_count = usize::from(context.code.registers_size);
    let incoming_count = usize::from(context.code.ins_size);
    let mut frame = RegisterFrame {
        registers: vec![RegisterType::Unknown; register_count],
    };
    let mut cursor = register_count.checked_sub(incoming_count).ok_or_else(|| {
        Error::invalid_instruction(0, "incoming register window exceeds the method frame")
    })?;
    if !context
        .declaration
        .access_flags
        .contains(AccessFlags::STATIC)
    {
        let receiver = if context.is_constructor() {
            RegisterType::Reference(ReferenceType::UninitializedThis {
                descriptor: context.owner.to_owned(),
            })
        } else {
            RegisterType::Reference(ReferenceType::Descriptor(context.owner.to_owned()))
        };
        write_value(&mut frame, cursor, receiver)?;
        cursor += 1;
    }
    for &parameter in &context.prototype.parameters {
        let descriptor = context.file.type_descriptor(parameter)?;
        let value = descriptor_type(descriptor, 0)?.ok_or_else(|| {
            Error::invalid_instruction(0, "method parameter cannot have void type")
        })?;
        let width = register_width(&value);
        write_value(&mut frame, cursor, value)?;
        cursor = cursor
            .checked_add(width)
            .ok_or_else(|| Error::invalid_instruction(0, "parameter register width overflowed"))?;
    }
    if cursor != register_count {
        return Err(Error::invalid_instruction(
            0,
            format!("method prototype ends at register {cursor}, expected {register_count}"),
        ));
    }
    Ok(frame)
}

pub(super) fn register_width(value: &RegisterType) -> usize {
    usize::from(value.is_wide_base()) + 1
}

pub(super) fn write_value(
    frame: &mut RegisterFrame,
    index: usize,
    value: RegisterType,
) -> Result<()> {
    let width = register_width(&value);
    let end = index
        .checked_add(width)
        .ok_or_else(|| Error::invalid_instruction(u32::MAX, "register write range overflowed"))?;
    if end > frame.registers.len() {
        return Err(Error::invalid_instruction(
            u32::MAX,
            "register write exceeds the method frame",
        ));
    }
    invalidate_intersecting_wide_values(frame, index, end);
    frame.registers[index] = value;
    if width == 2 {
        frame.registers[index + 1] = RegisterType::WideContinuation;
    }
    Ok(())
}

fn invalidate_intersecting_wide_values(frame: &mut RegisterFrame, start: usize, end: usize) {
    let mut bases = Vec::new();
    for position in 0..frame.registers.len() {
        if frame.registers[position].is_wide_base()
            && ranges_intersect(position, position.saturating_add(2), start, end)
        {
            bases.push(position);
        }
    }
    for base in bases {
        frame.registers[base] = RegisterType::Unknown;
        if let Some(continuation) = frame.registers.get_mut(base + 1) {
            *continuation = RegisterType::Unknown;
        }
    }
}

const fn ranges_intersect(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
}

fn merge_frames(
    current: &mut RegisterFrame,
    incoming: &RegisterFrame,
    hierarchy: &dyn ReferenceHierarchy,
) -> bool {
    let mut changed = false;
    for (current_value, incoming_value) in current.registers.iter_mut().zip(&incoming.registers) {
        let merged = merge_type(current_value, incoming_value, hierarchy);
        if *current_value != merged {
            *current_value = merged;
            changed = true;
        }
    }
    changed |= normalize_wide_pairs(current);
    changed
}

fn merge_type(
    left: &RegisterType,
    right: &RegisterType,
    hierarchy: &dyn ReferenceHierarchy,
) -> RegisterType {
    use RegisterType as R;
    if left == right {
        return left.clone();
    }
    match (left, right) {
        (R::Conflict | R::Unknown, _) | (_, R::Conflict | R::Unknown) => R::Conflict,
        (R::Zero, value) | (value, R::Zero)
            if matches!(
                value,
                R::Single
                    | R::Integer
                    | R::Float
                    | R::Reference(ReferenceType::Any | ReferenceType::Descriptor(_))
            ) =>
        {
            value.clone()
        }
        (R::WideZero, value) | (value, R::WideZero)
            if matches!(value, R::Wide | R::Long | R::Double) =>
        {
            value.clone()
        }
        (R::Single, R::Integer | R::Float) | (R::Integer | R::Float, R::Single) => R::Single,
        (R::Wide, R::Long | R::Double) | (R::Long | R::Double, R::Wide) => R::Wide,
        (R::Reference(left), R::Reference(right)) => merge_references(left, right, hierarchy),
        _ => R::Conflict,
    }
}

fn merge_references(
    left: &ReferenceType,
    right: &ReferenceType,
    hierarchy: &dyn ReferenceHierarchy,
) -> RegisterType {
    match (left, right) {
        (ReferenceType::Any, _) | (_, ReferenceType::Any) => {
            RegisterType::Reference(ReferenceType::Any)
        }
        (ReferenceType::Descriptor(left), ReferenceType::Descriptor(right)) => hierarchy
            .common_supertype(left, right)
            .map_or(RegisterType::Conflict, |descriptor| {
                RegisterType::Reference(ReferenceType::Descriptor(descriptor))
            }),
        _ => RegisterType::Conflict,
    }
}

fn normalize_wide_pairs(frame: &mut RegisterFrame) -> bool {
    let mut changed = false;
    for position in 0..frame.registers.len() {
        let invalid_base = frame.registers[position].is_wide_base()
            && frame.registers.get(position + 1) != Some(&RegisterType::WideContinuation);
        let invalid_continuation = frame.registers[position] == RegisterType::WideContinuation
            && position
                .checked_sub(1)
                .and_then(|previous| frame.registers.get(previous))
                .is_none_or(|value| !value.is_wide_base());
        if invalid_base || invalid_continuation {
            frame.registers[position] = RegisterType::Conflict;
            changed = true;
        }
    }
    changed
}
