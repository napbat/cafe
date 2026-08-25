//! Method entry construction and fixed-point JVM frame analysis.

use std::collections::BTreeMap;

use disassembler::cfglib::{
    Direction, EdgeRef, RootedGraphView, TryEdgeProblem, TrySolveError, try_solve_edge_problem_from,
};

use crate::bytecode::{Instruction, Opcode, Operand, decode_code};
use crate::classfile::{ClassFile, CodeAttribute, ConstantPool, MethodAccessFlags, MethodInfo};
use crate::descriptor::{self, JavaType, MethodDescriptor};
use crate::{Error, Result};

use super::flow::build_control_flow;
use super::hierarchy::{ClassHierarchy, JAVA_LANG_OBJECT_NAME, ReferenceHierarchy};
use super::model::{ControlFlow, FlowEdge, FlowEdgeKind, FrameState, FrameValue, MethodAnalysis};
use super::reference::resolve_instruction_reference;
use super::transfer::transfer;

const JAVA_LANG_THROWABLE_NAME: &str = "java/lang/Throwable";

pub(super) struct MethodContext<'a> {
    pub(super) pool: &'a ConstantPool,
    pub(super) owner: &'a str,
    pub(super) name: &'a str,
    pub(super) descriptor: &'a MethodDescriptor,
    pub(super) instructions: &'a BTreeMap<usize, &'a Instruction>,
    pub(super) hierarchy: &'a dyn ReferenceHierarchy,
}

/// Analyzes the sole code attribute of one class method, if present.
///
/// Existing `max_stack` and `max_locals` values must be at least the computed
/// requirements. Abstract and native methods without code return `None`.
///
/// # Errors
///
/// Returns an overload-qualified error for malformed bytecode, incompatible
/// frames, invalid constant references, or understated resource maxima.
pub fn analyze_method(class: &ClassFile, method: &MethodInfo) -> Result<Option<MethodAnalysis>> {
    let hierarchy = ClassHierarchy::from_classes([class])?;
    analyze_method_with_hierarchy(class, method, &hierarchy)
}

/// Analyzes one class method using caller-supplied classpath relationships.
///
/// Existing `max_stack` and `max_locals` values must be at least the computed
/// requirements. Abstract and native methods without code return `None`.
///
/// # Errors
///
/// Returns an overload-qualified error for malformed bytecode, incompatible
/// frames or references, invalid constant references, or understated maxima.
pub fn analyze_method_with_hierarchy(
    class: &ClassFile,
    method: &MethodInfo,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<Option<MethodAnalysis>> {
    let Some(code) = method.code() else {
        return Ok(None);
    };
    let owner = class.class_name()?.to_owned();
    let name = method.name(&class.constant_pool)?.to_owned();
    let descriptor = method.descriptor(&class.constant_pool)?.to_owned();
    let analysis = analyze_code_with_hierarchy(
        &class.constant_pool,
        &owner,
        &name,
        &descriptor,
        method.access_flags,
        code,
        hierarchy,
    )?;
    if code.max_stack < analysis.max_stack() {
        return Err(Error::invalid_bytecode(
            0,
            format!(
                "max_stack {} is smaller than computed requirement {}",
                code.max_stack,
                analysis.max_stack()
            ),
        )
        .in_class_method(owner, name, descriptor));
    }
    if code.max_locals < analysis.max_locals() {
        return Err(Error::invalid_bytecode(
            0,
            format!(
                "max_locals {} is smaller than computed requirement {}",
                code.max_locals,
                analysis.max_locals()
            ),
        )
        .in_class_method(owner, name, descriptor));
    }
    Ok(Some(analysis))
}

/// Computes exact stack/local maxima and verification frames for a code attribute.
///
/// The attribute's existing maxima do not constrain this calculation, allowing
/// callers to derive them for newly built code before installation.
///
/// # Errors
///
/// Returns an overload-qualified error for malformed bytecode, invalid constant
/// references, unsupported legacy subroutines, or incompatible frame merges.
pub fn analyze_code(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    code: &CodeAttribute,
) -> Result<MethodAnalysis> {
    let hierarchy = ClassHierarchy::new();
    analyze_code_with_hierarchy(
        pool,
        owner,
        name,
        descriptor,
        access_flags,
        code,
        &hierarchy,
    )
}

/// Computes JVM frames using caller-supplied classpath relationships.
///
/// The hierarchy is used for reference assignment and common-supertype merges.
/// This path is appropriate when generated code uses subclasses or interfaces
/// that are not declared by the class currently being assembled.
///
/// # Errors
///
/// Returns an overload-qualified error for malformed bytecode, invalid constant
/// references, unsupported legacy subroutines, or incompatible frame merges.
#[allow(clippy::too_many_arguments)]
pub fn analyze_code_with_hierarchy(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &str,
    access_flags: MethodAccessFlags,
    code: &CodeAttribute,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<MethodAnalysis> {
    let parsed = descriptor::parse_method(descriptor)
        .map_err(|error| error.in_class_method(owner, name, descriptor))?;
    let instructions =
        decode_code(code).map_err(|error| error.in_class_method(owner, name, descriptor))?;
    analyze_inner(
        pool,
        owner,
        name,
        &parsed,
        access_flags,
        code,
        &instructions,
        hierarchy,
    )
    .map_err(|error| error.in_class_method(owner, name, descriptor))
}

#[allow(clippy::too_many_arguments)]
fn analyze_inner(
    pool: &ConstantPool,
    owner: &str,
    name: &str,
    descriptor: &MethodDescriptor,
    access_flags: MethodAccessFlags,
    code: &CodeAttribute,
    instructions: &[Instruction],
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<MethodAnalysis> {
    if instructions
        .iter()
        .any(|instruction| matches!(instruction.opcode, Opcode::Jsr | Opcode::JsrW | Opcode::Ret))
    {
        return Err(Error::invalid_bytecode(
            0,
            "legacy jsr/ret subroutines are not supported by frame analysis",
        ));
    }
    for instruction in instructions {
        let _ = resolve_instruction_reference(pool, instruction)?;
    }
    let max_locals = required_locals(descriptor, access_flags, instructions)?;
    let flow = build_control_flow(instructions, &code.exception_table)?;
    let instruction_map = instructions
        .iter()
        .map(|instruction| (instruction.offset, instruction))
        .collect::<BTreeMap<_, _>>();
    let context = MethodContext {
        pool,
        owner,
        name,
        descriptor,
        instructions: &instruction_map,
        hierarchy,
    };
    solve(&context, access_flags, max_locals, flow)
}

fn solve(
    context: &MethodContext<'_>,
    access_flags: MethodAccessFlags,
    max_locals: u16,
    flow: ControlFlow,
) -> Result<MethodAnalysis> {
    let entry = flow.root();
    let initial = initial_frame(context, access_flags, max_locals)?;
    let problem = FrameProblem {
        context,
        entry,
        initial,
    };
    let facts =
        try_solve_edge_problem_from(&flow, &problem, &[entry]).map_err(|error| match error {
            TrySolveError::Problem(error) => error,
            TrySolveError::Solver(error) => {
                Error::invalid_bytecode(flow.entry(), error.to_string())
            }
        })?;

    let mut entries = BTreeMap::new();
    let mut exits = BTreeMap::new();
    let mut maximum_stack_slots = 0;
    for (node, &offset) in flow.nodes().iter().enumerate() {
        if let Some(frame) = facts.fact_in(node) {
            maximum_stack_slots = maximum_stack_slots.max(frame.stack_slots());
            entries.insert(offset, frame.clone());
        }
        if let Some(frame) = facts.fact_out(node) {
            maximum_stack_slots = maximum_stack_slots.max(frame.stack_slots());
            exits.insert(offset, frame.clone());
        }
    }

    if let Some(unreachable) = flow
        .nodes()
        .iter()
        .find(|offset| !entries.contains_key(offset))
    {
        return Err(Error::invalid_bytecode(
            *unreachable,
            "unreachable code requires an explicit seed frame and cannot be inferred",
        ));
    }

    let max_stack = u16::try_from(maximum_stack_slots)
        .map_err(|_| Error::invalid_bytecode(0, "operand stack exceeds u16 slots"))?;
    Ok(MethodAnalysis {
        flow,
        entries,
        exits,
        max_stack,
        max_locals,
    })
}

struct FrameProblem<'context, 'method> {
    context: &'context MethodContext<'method>,
    entry: usize,
    initial: FrameState,
}

impl TryEdgeProblem<ControlFlow> for FrameProblem<'_, '_> {
    type Fact = Option<FrameState>;
    type Error = Error;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &ControlFlow) -> Self::Fact {
        None
    }

    fn boundary(&self, _graph: &ControlFlow, node: usize) -> Result<Option<Self::Fact>> {
        Ok((node == self.entry).then(|| Some(self.initial.clone())))
    }

    fn meet(
        &self,
        graph: &ControlFlow,
        node: usize,
        left: &Self::Fact,
        right: &Self::Fact,
    ) -> Result<Self::Fact> {
        match (left, right) {
            (None, None) => Ok(None),
            (Some(frame), None) | (None, Some(frame)) => Ok(Some(frame.clone())),
            (Some(left), Some(right)) => {
                let mut merged = left.clone();
                let _ = merge_frames(
                    &mut merged,
                    right,
                    graph.node_offset(node),
                    self.context.hierarchy,
                )?;
                Ok(Some(merged))
            }
        }
    }

    fn transfer_node(
        &self,
        graph: &ControlFlow,
        node: usize,
        input: &Self::Fact,
    ) -> Result<Self::Fact> {
        let Some(input) = input else {
            return Ok(None);
        };
        let offset = graph.node_offset(node);
        transfer(self.context, self.context.instructions[&offset], input).map(Some)
    }

    fn transfer_edge(
        &self,
        _graph: &ControlFlow,
        edge: EdgeRef<'_, usize, usize, FlowEdge>,
        node_input: &Self::Fact,
        node_output: &Self::Fact,
    ) -> Result<Self::Fact> {
        match edge.data().kind {
            FlowEdgeKind::Exception { catch_type } => {
                node_input.as_ref().map_or(Ok(None), |input| {
                    exception_frame(self.context.pool, input, catch_type).map(Some)
                })
            }
            FlowEdgeKind::FallThrough | FlowEdgeKind::Branch => Ok(node_output.clone()),
        }
    }
}

fn initial_frame(
    context: &MethodContext<'_>,
    access_flags: MethodAccessFlags,
    max_locals: u16,
) -> Result<FrameState> {
    let mut frame = FrameState {
        locals: vec![FrameValue::Top; usize::from(max_locals)],
        stack: Vec::new(),
    };
    let mut cursor = 0;
    if !access_flags.contains(MethodAccessFlags::STATIC) {
        let receiver = if context.name == crate::classfile::INSTANCE_INITIALIZER_NAME
            && context.owner != JAVA_LANG_OBJECT_NAME
        {
            FrameValue::UninitializedThis
        } else {
            FrameValue::Reference(context.owner.to_owned())
        };
        write_local(&mut frame, cursor, receiver, 0)?;
        cursor += 1;
    }
    for parameter in &context.descriptor.parameters {
        let value = frame_value(parameter);
        let width = value.slot_count();
        write_local(&mut frame, cursor, value, 0)?;
        cursor += width;
    }
    Ok(frame)
}

fn required_locals(
    descriptor: &MethodDescriptor,
    access_flags: MethodAccessFlags,
    instructions: &[Instruction],
) -> Result<u16> {
    let initial = descriptor.parameters.iter().try_fold(
        usize::from(!access_flags.contains(MethodAccessFlags::STATIC)),
        |count, parameter| {
            count
                .checked_add(parameter.slot_width().slot_count())
                .ok_or_else(|| Error::invalid_bytecode(0, "parameter local width overflowed"))
        },
    )?;
    let required = instructions.iter().filter_map(local_access).try_fold(
        initial,
        |maximum, (index, width)| {
            usize::from(index)
                .checked_add(width)
                .map(|end| maximum.max(end))
                .ok_or_else(|| Error::invalid_bytecode(0, "local-variable index range overflowed"))
        },
    )?;
    u16::try_from(required)
        .map_err(|_| Error::invalid_bytecode(0, "local-variable frame exceeds u16 slots"))
}

fn local_access(instruction: &Instruction) -> Option<(u16, usize)> {
    let explicit = match instruction.operand {
        Operand::Local(index) | Operand::Increment { index, .. } => Some(index),
        _ => None,
    };
    let index = explicit.or_else(|| compact_local(instruction.opcode))?;
    let width = if matches!(
        instruction.opcode,
        Opcode::LLoad
            | Opcode::LLoad0
            | Opcode::LLoad1
            | Opcode::LLoad2
            | Opcode::LLoad3
            | Opcode::DLoad
            | Opcode::DLoad0
            | Opcode::DLoad1
            | Opcode::DLoad2
            | Opcode::DLoad3
            | Opcode::LStore
            | Opcode::LStore0
            | Opcode::LStore1
            | Opcode::LStore2
            | Opcode::LStore3
            | Opcode::DStore
            | Opcode::DStore0
            | Opcode::DStore1
            | Opcode::DStore2
            | Opcode::DStore3
    ) {
        2
    } else {
        1
    };
    Some((index, width))
}

pub(super) const fn compact_local(opcode: Opcode) -> Option<u16> {
    use Opcode as O;
    match opcode {
        O::ILoad0
        | O::LLoad0
        | O::FLoad0
        | O::DLoad0
        | O::ALoad0
        | O::IStore0
        | O::LStore0
        | O::FStore0
        | O::DStore0
        | O::AStore0 => Some(0),
        O::ILoad1
        | O::LLoad1
        | O::FLoad1
        | O::DLoad1
        | O::ALoad1
        | O::IStore1
        | O::LStore1
        | O::FStore1
        | O::DStore1
        | O::AStore1 => Some(1),
        O::ILoad2
        | O::LLoad2
        | O::FLoad2
        | O::DLoad2
        | O::ALoad2
        | O::IStore2
        | O::LStore2
        | O::FStore2
        | O::DStore2
        | O::AStore2 => Some(2),
        O::ILoad3
        | O::LLoad3
        | O::FLoad3
        | O::DLoad3
        | O::ALoad3
        | O::IStore3
        | O::LStore3
        | O::FStore3
        | O::DStore3
        | O::AStore3 => Some(3),
        _ => None,
    }
}

pub(super) fn local_index(instruction: &Instruction) -> Result<u16> {
    match instruction.operand {
        Operand::Local(index) | Operand::Increment { index, .. } => Ok(index),
        _ => compact_local(instruction.opcode).ok_or_else(|| {
            Error::invalid_bytecode(instruction.offset, "local-variable index is missing")
        }),
    }
}

pub(super) fn write_local(
    frame: &mut FrameState,
    index: usize,
    value: FrameValue,
    offset: usize,
) -> Result<()> {
    let width = value.slot_count();
    let end = index
        .checked_add(width)
        .ok_or_else(|| Error::invalid_bytecode(offset, "local write range overflowed"))?;
    if end > frame.locals.len() {
        return Err(Error::invalid_bytecode(
            offset,
            format!("local write {index}..{end} exceeds computed max_locals"),
        ));
    }
    invalidate_wide_locals(frame, index, end);
    frame.locals[index] = value;
    if width == 2 {
        frame.locals[index + 1] = FrameValue::WideContinuation;
    }
    Ok(())
}

fn invalidate_wide_locals(frame: &mut FrameState, start: usize, end: usize) {
    let bases = frame
        .locals
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            (value.is_category_two() && index < end && start < index.saturating_add(2))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    for base in bases {
        frame.locals[base] = FrameValue::Top;
        if let Some(continuation) = frame.locals.get_mut(base + 1) {
            *continuation = FrameValue::Top;
        }
    }
}

fn exception_frame(pool: &ConstantPool, entry: &FrameState, catch_type: u16) -> Result<FrameState> {
    let class = if catch_type == crate::classfile::CATCH_ALL_EXCEPTION_INDEX {
        JAVA_LANG_THROWABLE_NAME.to_owned()
    } else {
        pool.class_name(catch_type)?.to_owned()
    };
    Ok(FrameState {
        locals: entry.locals.clone(),
        stack: vec![FrameValue::Reference(class)],
    })
}

fn merge_frames(
    current: &mut FrameState,
    incoming: &FrameState,
    offset: usize,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<bool> {
    if current.stack.len() != incoming.stack.len() {
        return Err(Error::invalid_bytecode(
            offset,
            format!(
                "predecessors provide stack heights {} and {}",
                current.stack.len(),
                incoming.stack.len()
            ),
        ));
    }
    let mut changed = false;
    for (left, right) in current.locals.iter_mut().zip(&incoming.locals) {
        let merged = merge_local(left, right, hierarchy);
        if *left != merged {
            *left = merged;
            changed = true;
        }
    }
    changed |= normalize_wide_locals(current);
    for (left, right) in current.stack.iter_mut().zip(&incoming.stack) {
        let merged = merge_stack(left, right, offset, hierarchy)?;
        if *left != merged {
            *left = merged;
            changed = true;
        }
    }
    Ok(changed)
}

fn merge_local(
    left: &FrameValue,
    right: &FrameValue,
    hierarchy: &dyn ReferenceHierarchy,
) -> FrameValue {
    if left == right {
        return left.clone();
    }
    match (left, right) {
        (FrameValue::Top | FrameValue::WideContinuation, _)
        | (_, FrameValue::Top | FrameValue::WideContinuation) => FrameValue::Top,
        _ => merge_reference(left, right, hierarchy).unwrap_or(FrameValue::Top),
    }
}

fn merge_stack(
    left: &FrameValue,
    right: &FrameValue,
    offset: usize,
    hierarchy: &dyn ReferenceHierarchy,
) -> Result<FrameValue> {
    if left == right {
        return Ok(left.clone());
    }
    merge_reference(left, right, hierarchy).ok_or_else(|| {
        Error::invalid_bytecode(
            offset,
            format!("predecessor stack types {left:?} and {right:?} are incompatible"),
        )
    })
}

fn merge_reference(
    left: &FrameValue,
    right: &FrameValue,
    hierarchy: &dyn ReferenceHierarchy,
) -> Option<FrameValue> {
    match (left, right) {
        (FrameValue::Null, FrameValue::Reference(name))
        | (FrameValue::Reference(name), FrameValue::Null) => {
            Some(FrameValue::Reference(name.clone()))
        }
        (FrameValue::Reference(left), FrameValue::Reference(right)) => hierarchy
            .common_supertype(left, right)
            .map(FrameValue::Reference),
        _ => None,
    }
}

fn normalize_wide_locals(frame: &mut FrameState) -> bool {
    let mut changed = false;
    for index in 0..frame.locals.len() {
        let invalid_base = frame.locals[index].is_category_two()
            && frame.locals.get(index + 1) != Some(&FrameValue::WideContinuation);
        let invalid_continuation = frame.locals[index] == FrameValue::WideContinuation
            && index
                .checked_sub(1)
                .and_then(|previous| frame.locals.get(previous))
                .is_none_or(|value| !value.is_category_two());
        if invalid_base || invalid_continuation {
            frame.locals[index] = FrameValue::Top;
            changed = true;
        }
    }
    changed
}

pub(super) fn frame_value(value: &JavaType) -> FrameValue {
    match value {
        JavaType::Byte | JavaType::Char | JavaType::Int | JavaType::Short | JavaType::Boolean => {
            FrameValue::Integer
        }
        JavaType::Float => FrameValue::Float,
        JavaType::Long => FrameValue::Long,
        JavaType::Double => FrameValue::Double,
        JavaType::Object(name) => FrameValue::Reference(name.clone()),
        JavaType::Array(_) => FrameValue::Reference(descriptor_text(value)),
    }
}

fn descriptor_text(value: &JavaType) -> String {
    match value {
        JavaType::Byte => "B".to_owned(),
        JavaType::Char => "C".to_owned(),
        JavaType::Double => "D".to_owned(),
        JavaType::Float => "F".to_owned(),
        JavaType::Int => "I".to_owned(),
        JavaType::Long => "J".to_owned(),
        JavaType::Short => "S".to_owned(),
        JavaType::Boolean => "Z".to_owned(),
        JavaType::Object(name) => format!("L{name};"),
        JavaType::Array(component) => format!("[{}", descriptor_text(component)),
    }
}
