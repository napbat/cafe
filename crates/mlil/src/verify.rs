//! Strict structural and semantic MLIL verification.

mod arrays;

use disassembler::cfglib::EdgeId;
use disassembler::{BinaryFormat, CatchType, Reference, ReferenceKind, ReferenceSymbol};

use crate::VerificationIssue;
use crate::descriptor::{self, MethodDescriptor};
use crate::model::{
    AllocationKind, ArrayAccess, BranchOperandKind, CallKind, Constant, ControlClass, Conversion,
    EdgeRole, ElementType, EntityId, FieldAccess, Function, Instruction, Operation, Relation,
    ValueType,
};

use self::arrays::{valid_array_allocation, valid_array_initialization, valid_initialized_array};

pub(crate) fn verify_function(function: &Function, issues: &mut Vec<VerificationIssue>) {
    let method_descriptor = descriptor::method_descriptor(&function.source().symbol.signature);
    if method_descriptor.is_none() {
        issue(
            issues,
            format!(
                "function {} has an invalid method descriptor",
                function.source()
            ),
        );
    }
    verify_variables(function, issues);
    verify_instructions(function, method_descriptor.as_ref(), issues);
    verify_edges(function, issues);
    verify_blocks(function, issues);
}

fn verify_variables(function: &Function, issues: &mut Vec<VerificationIssue>) {
    // Several variables may share one native storage: lifetime splitting
    // legitimately produces multiple variables carrying the same slot's
    // provenance. Only the format of that provenance is constrained.
    for variable in function.variables() {
        if let Some(native) = variable.native
            && native.format != function.source().format
        {
            issue(
                issues,
                format!(
                    "variable {} uses native format {} in a {} function",
                    variable.id,
                    native.format,
                    function.source().format
                ),
            );
        }
    }
}

fn verify_instructions(
    function: &Function,
    method_descriptor: Option<&MethodDescriptor>,
    issues: &mut Vec<VerificationIssue>,
) {
    for block in function.cfg().blocks() {
        for instruction in block.instructions() {
            verify_instruction(function, instruction, method_descriptor, issues);
        }
    }
}

fn verify_instruction(
    function: &Function,
    instruction: &Instruction,
    method_descriptor: Option<&MethodDescriptor>,
    issues: &mut Vec<VerificationIssue>,
) {
    if matches!(instruction.operation(), Operation::Select) {
        issue(
            issues,
            format!(
                "instruction {} uses HLIL-only select vocabulary in canonical MLIL",
                instruction.id()
            ),
        );
    }
    for value_type in instruction
        .use_types()
        .iter()
        .chain(instruction.def_types())
    {
        if !valid_value_type(function.source().format, value_type) {
            issue(
                issues,
                format!(
                    "{} contains invalid value type {value_type:?}",
                    instruction.id()
                ),
            );
        }
    }
    verify_operation(instruction, method_descriptor, issues);
}

fn valid_value_type(format: BinaryFormat, value_type: &ValueType) -> bool {
    match value_type {
        ValueType::Reference(Some(descriptor)) => descriptor::is_reference(descriptor),
        ValueType::UninitializedThis(descriptor) => descriptor::is_object(descriptor),
        ValueType::Uninitialized { descriptor, site } => {
            descriptor::is_object(descriptor) && site.format == format
        }
        ValueType::ReturnAddress => format == BinaryFormat::JavaClass,
        _ => true,
    }
}

fn verify_operation(
    instruction: &Instruction,
    method_descriptor: Option<&MethodDescriptor>,
    issues: &mut Vec<VerificationIssue>,
) {
    let uses = instruction.use_types();
    let defs = instruction.def_types();
    let id = instruction.id();
    if !valid_operation_signature(instruction.operation(), uses, defs, method_descriptor) {
        issue(
            issues,
            format!(
                "{id} has an invalid typed signature for {} ({} uses, {} definitions)",
                instruction.operation().mnemonic(),
                uses.len(),
                defs.len()
            ),
        );
    }
}

fn valid_operation_signature(
    operation: &Operation,
    uses: &[ValueType],
    defs: &[ValueType],
    method_descriptor: Option<&MethodDescriptor>,
) -> bool {
    match operation {
        Operation::Nop
        | Operation::Copy
        | Operation::ParallelCopy
        | Operation::Discard
        | Operation::TypeRefine
        | Operation::Constant(_)
        | Operation::Unary(_)
        | Operation::Binary(_)
        | Operation::Convert(_)
        | Operation::Compare(_) => valid_value_operation(operation, uses, defs),
        Operation::Branch(_)
        | Operation::Jump
        | Operation::Switch(_)
        | Operation::Return
        | Operation::Throw => valid_control_operation(operation, uses, defs, method_descriptor),
        _ => valid_managed_operation(operation, uses, defs),
    }
}

fn valid_value_operation(operation: &Operation, uses: &[ValueType], defs: &[ValueType]) -> bool {
    match operation {
        Operation::Nop => counts(uses, defs, 0, 0),
        Operation::Copy => counts(uses, defs, 1, 1) && defs[0].accepts(&uses[0]),
        Operation::ParallelCopy => {
            !uses.is_empty()
                && uses.len() == defs.len()
                && defs
                    .iter()
                    .zip(uses)
                    .all(|(definition, usage)| definition.accepts(usage))
        }
        Operation::Discard => !uses.is_empty() && defs.is_empty(),
        Operation::TypeRefine => {
            !uses.is_empty()
                && uses.len() == defs.len()
                && uses.iter().zip(defs).all(|(before, after)| {
                    matches!(
                        before,
                        ValueType::UninitializedThis(_) | ValueType::Uninitialized { .. }
                    ) && matches!(after, ValueType::Reference(_))
                        && same_uninitialized_descriptor(before, after)
                })
        }
        Operation::Constant(constant) => {
            counts(uses, defs, 0, 1) && constant_type(constant).accepts(&defs[0])
        }
        Operation::Unary(operator) => {
            counts(uses, defs, 1, 1)
                && match operator {
                    crate::UnaryOperator::Negate => arithmetic(&uses[0]),
                    crate::UnaryOperator::BitwiseNot => integral(&uses[0]),
                }
                && defs[0].accepts(&uses[0])
        }
        Operation::Binary(operator) => {
            counts(uses, defs, 2, 1)
                && binary_operands(*operator, &uses[0], &uses[1])
                && defs[0].accepts(&uses[0])
        }
        Operation::Convert(conversion) => {
            let (source, target) = conversion_types(*conversion);
            counts(uses, defs, 1, 1) && source.accepts(&uses[0]) && target.accepts(&defs[0])
        }
        Operation::Compare(comparison) => {
            let operand = match comparison {
                crate::ThreeWayComparison::Long => ValueType::Long,
                crate::ThreeWayComparison::FloatNanLow
                | crate::ThreeWayComparison::FloatNanHigh => ValueType::Float,
                crate::ThreeWayComparison::DoubleNanLow
                | crate::ThreeWayComparison::DoubleNanHigh => ValueType::Double,
            };
            counts(uses, defs, 2, 1)
                && operand.accepts(&uses[0])
                && operand.accepts(&uses[1])
                && integer_like(&defs[0])
        }
        _ => false,
    }
}

fn valid_control_operation(
    operation: &Operation,
    uses: &[ValueType],
    defs: &[ValueType],
    method_descriptor: Option<&MethodDescriptor>,
) -> bool {
    match operation {
        Operation::Branch(predicate) => match predicate.operands {
            BranchOperandKind::IntegerZero | BranchOperandKind::Boolean => {
                counts(uses, defs, 1, 0)
                    && integer_like(&uses[0])
                    && (predicate.operands != BranchOperandKind::Boolean
                        || equality_relation(predicate.relation))
            }
            BranchOperandKind::IntegerPair => {
                counts(uses, defs, 2, 0) && integer_like(&uses[0]) && integer_like(&uses[1])
            }
            BranchOperandKind::ReferencePair => {
                counts(uses, defs, 2, 0)
                    && equality_relation(predicate.relation)
                    && reference_like(&uses[0])
                    && reference_like(&uses[1])
            }
            BranchOperandKind::ReferenceNull => {
                counts(uses, defs, 1, 0)
                    && equality_relation(predicate.relation)
                    && reference_like(&uses[0])
            }
        },
        Operation::Jump => counts(uses, defs, 0, 0),
        Operation::Switch(keys) => {
            counts(uses, defs, 1, 0)
                && integer_like(&uses[0])
                && keys.windows(2).all(|pair| pair[0] < pair[1])
        }
        Operation::Return => valid_return(method_descriptor, uses, defs),
        Operation::Throw => counts(uses, defs, 1, 0) && reference_like(&uses[0]),
        _ => false,
    }
}

fn valid_managed_operation(operation: &Operation, uses: &[ValueType], defs: &[ValueType]) -> bool {
    match operation {
        Operation::Array { access, element } => match access {
            ArrayAccess::Get => {
                counts(uses, defs, 2, 1)
                    && reference_like(&uses[0])
                    && integer_like(&uses[1])
                    && element_type(*element).accepts(&defs[0])
            }
            ArrayAccess::Put => {
                counts(uses, defs, 3, 0)
                    && reference_like(&uses[0])
                    && integer_like(&uses[1])
                    && element_type(*element).accepts(&uses[2])
            }
        },
        Operation::ArrayLength => {
            counts(uses, defs, 1, 1) && reference_like(&uses[0]) && integer_like(&defs[0])
        }
        Operation::Field { access, field } => valid_field(*access, field, uses, defs),
        Operation::Call {
            kind,
            target,
            descriptor,
        } => valid_call(*kind, target, descriptor.as_deref(), uses, defs),
        Operation::Allocate(kind) => match kind {
            AllocationKind::Object(reference) => {
                valid_type_reference(reference)
                    && counts(uses, defs, 0, 1)
                    && reference_like(&defs[0])
            }
            AllocationKind::Array {
                array_type,
                dimensions,
            } => valid_array_allocation(array_type, *dimensions, uses, defs),
            AllocationKind::InitializedArray { array_type } => {
                valid_initialized_array(array_type, uses, defs)
            }
        },
        Operation::InitializeArray { array_type, values } => {
            valid_array_initialization(array_type, values, uses, defs)
        }
        Operation::Monitor(_) => counts(uses, defs, 1, 0) && reference_like(&uses[0]),
        Operation::CheckCast(reference) => {
            valid_type_reference(reference)
                && counts(uses, defs, 1, 1)
                && reference_like(&uses[0])
                && reference_like(&defs[0])
        }
        Operation::InstanceOf(reference) => {
            valid_type_reference(reference)
                && counts(uses, defs, 1, 1)
                && reference_like(&uses[0])
                && integer_like(&defs[0])
        }
        Operation::CaughtException(catch) => {
            counts(uses, defs, 0, 1) && valid_caught_type(catch, &defs[0])
        }
        Operation::Intrinsic(name) => !name.is_empty(),
        _ => false,
    }
}

fn counts(uses: &[ValueType], defs: &[ValueType], use_count: usize, def_count: usize) -> bool {
    uses.len() == use_count && defs.len() == def_count
}

fn same_uninitialized_descriptor(before: &ValueType, after: &ValueType) -> bool {
    let (ValueType::UninitializedThis(descriptor) | ValueType::Uninitialized { descriptor, .. }) =
        before
    else {
        return false;
    };
    match after {
        ValueType::Reference(None) => true,
        ValueType::Reference(Some(initialized)) => initialized == descriptor,
        _ => false,
    }
}

fn binary_operands(operator: crate::BinaryOperator, left: &ValueType, right: &ValueType) -> bool {
    use crate::BinaryOperator::{
        Add, And, Divide, Multiply, Or, Remainder, ReverseSubtract, ShiftLeft, ShiftRight,
        Subtract, UnsignedShiftRight, Xor,
    };
    match operator {
        Add | Subtract | Multiply | Divide | Remainder => {
            arithmetic(left) && compatible(left, right)
        }
        ReverseSubtract => integer_like(left) && integer_like(right),
        And | Or | Xor => integral(left) && compatible(left, right),
        ShiftLeft | ShiftRight | UnsignedShiftRight => integral(left) && integer_like(right),
    }
}

fn compatible(left: &ValueType, right: &ValueType) -> bool {
    left.accepts(right) || right.accepts(left)
}

fn equality_relation(relation: Relation) -> bool {
    matches!(relation, Relation::Equal | Relation::NotEqual)
}

fn valid_return(method: Option<&MethodDescriptor>, uses: &[ValueType], defs: &[ValueType]) -> bool {
    if !defs.is_empty() {
        return false;
    }
    let Some(method) = method else {
        return uses.len() <= 1;
    };
    match &method.return_type {
        None => uses.is_empty(),
        Some(expected) => uses.len() == 1 && descriptor::accepts(expected, &uses[0]),
    }
}

fn valid_field(
    access: FieldAccess,
    field: &Reference,
    uses: &[ValueType],
    defs: &[ValueType],
) -> bool {
    if field.kind != ReferenceKind::Field {
        return false;
    }
    let shape = match access {
        FieldAccess::GetInstance => counts(uses, defs, 1, 1) && reference_like(&uses[0]),
        FieldAccess::PutInstance => counts(uses, defs, 2, 0) && reference_like(&uses[0]),
        FieldAccess::GetStatic => counts(uses, defs, 0, 1),
        FieldAccess::PutStatic => counts(uses, defs, 1, 0),
    };
    if !shape {
        return false;
    }
    let Some(symbol) = &field.symbol else {
        return true;
    };
    let ReferenceSymbol::Field { descriptor, .. } = symbol else {
        return false;
    };
    let Some(expected) = descriptor::field_type(descriptor) else {
        return false;
    };
    let value = match access {
        FieldAccess::GetInstance | FieldAccess::GetStatic => &defs[0],
        FieldAccess::PutInstance => &uses[1],
        FieldAccess::PutStatic => &uses[0],
    };
    descriptor::accepts(&expected, value)
}

fn valid_call(
    kind: CallKind,
    target: &Reference,
    operation_descriptor: Option<&str>,
    uses: &[ValueType],
    defs: &[ValueType],
) -> bool {
    let dynamic = kind == CallKind::Dynamic;
    if dynamic != (target.kind == ReferenceKind::DynamicCallSite)
        || (!dynamic
            && !matches!(
                target.kind,
                ReferenceKind::Method | ReferenceKind::InterfaceMethod
            ))
    {
        return false;
    }
    let symbol_descriptor = match &target.symbol {
        Some(ReferenceSymbol::Method { descriptor, .. }) => Some(descriptor.as_str()),
        Some(_) => return false,
        None => None,
    };
    if kind != CallKind::Polymorphic
        && let Some(symbol_descriptor) = symbol_descriptor
        && operation_descriptor != Some(symbol_descriptor)
    {
        return false;
    }
    let has_receiver = !matches!(kind, CallKind::Static | CallKind::Dynamic);
    let Some(operation_descriptor) = operation_descriptor else {
        return defs.len() <= 1 && (!has_receiver || uses.first().is_some_and(reference_like));
    };
    let Some(method) = descriptor::method_descriptor(operation_descriptor) else {
        return false;
    };
    if uses.len() != method.parameters.len() + usize::from(has_receiver)
        || (has_receiver && !reference_like(&uses[0]))
    {
        return false;
    }
    let parameter_start = usize::from(has_receiver);
    if !method
        .parameters
        .iter()
        .zip(&uses[parameter_start..])
        .all(|(expected, actual)| descriptor::accepts(expected, actual))
    {
        return false;
    }
    match method.return_type {
        None => defs.is_empty(),
        Some(expected) => defs.len() == 1 && descriptor::accepts(&expected, &defs[0]),
    }
}

fn valid_type_reference(reference: &Reference) -> bool {
    reference.kind == ReferenceKind::Type
        && match &reference.symbol {
            Some(ReferenceSymbol::Type(name)) => !name.is_empty(),
            Some(_) => false,
            None => true,
        }
}

fn valid_caught_type(catch: &CatchType, value: &ValueType) -> bool {
    if let CatchType::Type(descriptor) = catch
        && !descriptor::is_object(descriptor)
    {
        return false;
    }
    match value {
        ValueType::Unknown | ValueType::Reference(None) => true,
        ValueType::Reference(Some(actual)) => match catch {
            CatchType::Any => true,
            CatchType::Type(expected) => expected == actual,
        },
        _ => false,
    }
}

fn verify_edges(function: &Function, issues: &mut Vec<VerificationIssue>) {
    let edge_ids = function
        .cfg()
        .edges()
        .map(disassembler::cfglib::Edge::id)
        .collect::<Vec<_>>();
    for edge in edge_ids {
        verify_edge(function, edge, issues);
    }
}

fn verify_edge(function: &Function, edge_id: EdgeId, issues: &mut Vec<VerificationIssue>) {
    let edge = function.cfg().edge(edge_id);
    let metadata = edge.payload();
    if metadata.role == EdgeRole::Commit {
        verify_commit_edge(function, edge_id, issues);
    }
    match (&metadata.role, metadata.throw_site) {
        (
            EdgeRole::Exception {
                catch, protected, ..
            },
            Some(throw_site),
        ) => verify_exception_edge(function, edge_id, catch, protected, throw_site, issues),
        (EdgeRole::Exception { .. }, None) => issue(
            issues,
            format!("exception edge {edge_id} has no exact throw site"),
        ),
        (_, Some(throw_site)) => issue(
            issues,
            format!("ordinary edge {edge_id} carries throw site {throw_site}"),
        ),
        (_, None) => {}
    }
}

fn verify_commit_edge(function: &Function, edge_id: EdgeId, issues: &mut Vec<VerificationIssue>) {
    let edge = function.cfg().edge(edge_id);
    let source = function.cfg().block(edge.source());
    if !source
        .instructions()
        .last()
        .is_some_and(Instruction::may_throw)
    {
        issue(
            issues,
            format!("commit edge {edge_id} does not follow a throwing instruction"),
        );
    }
    if function.cfg().block(edge.target()).is_empty() {
        issue(
            issues,
            format!("commit edge {edge_id} targets an empty block"),
        );
    }
}

fn verify_exception_edge(
    function: &Function,
    edge_id: EdgeId,
    catch: &CatchType,
    protected: &disassembler::AddressRange,
    throw_site: crate::InstructionId,
    issues: &mut Vec<VerificationIssue>,
) {
    let edge = function.cfg().edge(edge_id);
    if protected.is_empty() {
        issue(
            issues,
            format!("edge {edge_id} has an empty protected range"),
        );
    }
    let source = function.cfg().block(edge.source());
    let source_contains_throw = source
        .instructions()
        .iter()
        .any(|instruction| instruction.id() == throw_site && instruction.may_throw());
    if !source_contains_throw {
        issue(
            issues,
            format!(
                "exception edge {edge_id} throw site {throw_site} is not a throwing instruction in {}",
                edge.source()
            ),
        );
    }
    if source
        .instructions()
        .last()
        .is_none_or(|instruction| instruction.id() != throw_site)
    {
        issue(
            issues,
            format!(
                "exception edge {edge_id} throw site {throw_site} is not terminal in {}",
                edge.source()
            ),
        );
    }
    let protected_throw = function
        .provenance()
        .mappings_to(EntityId::Instruction(throw_site))
        .any(|entry| entry.source.start >= protected.start && entry.source.end <= protected.end);
    if !protected_throw {
        issue(
            issues,
            format!(
                "exception edge {edge_id} throw site {throw_site} has no provenance inside {}..{}",
                protected.start, protected.end
            ),
        );
    }
    let landing_matches = function
        .cfg()
        .block(edge.target())
        .instructions()
        .first()
        .is_some_and(|instruction| {
            matches!(instruction.operation(), Operation::CaughtException(found) if found == catch)
        });
    if !landing_matches {
        issue(
            issues,
            format!("exception edge {edge_id} does not target a matching caught-exception landing"),
        );
    }
}

fn verify_blocks(function: &Function, issues: &mut Vec<VerificationIssue>) {
    let entry = function.cfg().entry();
    for block in function.cfg().blocks() {
        if block.id() == entry {
            continue;
        }
        let normal_roles: Vec<_> = function
            .cfg()
            .successor_edges(block.id())
            .iter()
            .map(|edge| &function.cfg().edge(*edge).payload().role)
            .filter(|role| !role.is_exception())
            .collect();
        let Some(last) = block.instructions().last() else {
            continue;
        };
        let valid = match last.operation().control_class() {
            ControlClass::Normal => {
                normal_roles.len() == 1
                    && matches!(normal_roles[0], EdgeRole::Commit | EdgeRole::FallThrough)
            }
            ControlClass::Branch => {
                normal_roles.len() == 2
                    && normal_roles.contains(&&EdgeRole::BranchTrue)
                    && normal_roles.contains(&&EdgeRole::BranchFalse)
            }
            ControlClass::Jump => normal_roles.len() == 1 && *normal_roles[0] == EdgeRole::Jump,
            ControlClass::Switch => {
                let Operation::Switch(keys) = last.operation() else {
                    unreachable!();
                };
                let defaults = normal_roles
                    .iter()
                    .filter(|role| ***role == EdgeRole::SwitchDefault)
                    .count();
                let mut cases: Vec<_> = normal_roles
                    .iter()
                    .filter_map(|role| match **role {
                        EdgeRole::SwitchCase(key) => Some(key),
                        _ => None,
                    })
                    .collect();
                cases.sort_unstable();
                defaults == 1 && cases.as_slice() == keys.as_slice()
            }
            ControlClass::Return | ControlClass::Throw => normal_roles.is_empty(),
        };
        if !valid {
            issue(
                issues,
                format!(
                    "block {} outgoing roles {:?} do not match terminal operation {}",
                    block.id(),
                    normal_roles,
                    last.operation().mnemonic()
                ),
            );
        }
    }
}

fn constant_type(constant: &Constant) -> ValueType {
    match constant {
        Constant::Null => ValueType::Null,
        Constant::Integer(_) => ValueType::Integer,
        Constant::Long(_) => ValueType::Long,
        Constant::Float(_) => ValueType::Float,
        Constant::Double(_) => ValueType::Double,
        Constant::Reference(reference) => match reference.kind {
            ReferenceKind::String
            | ReferenceKind::Type
            | ReferenceKind::MethodPrototype
            | ReferenceKind::MethodHandle => ValueType::Reference(None),
            ReferenceKind::Constant
            | ReferenceKind::Field
            | ReferenceKind::Method
            | ReferenceKind::InterfaceMethod
            | ReferenceKind::DynamicCallSite => ValueType::Unknown,
        },
    }
}

fn conversion_types(conversion: Conversion) -> (ValueType, ValueType) {
    use Conversion::{
        DoubleToFloat, DoubleToInt, DoubleToLong, FloatToDouble, FloatToInt, FloatToLong,
        IntToByte, IntToChar, IntToDouble, IntToFloat, IntToLong, IntToShort, LongToDouble,
        LongToFloat, LongToInt,
    };
    match conversion {
        IntToLong => (ValueType::Integer, ValueType::Long),
        IntToFloat => (ValueType::Integer, ValueType::Float),
        IntToDouble => (ValueType::Integer, ValueType::Double),
        LongToInt => (ValueType::Long, ValueType::Integer),
        LongToFloat => (ValueType::Long, ValueType::Float),
        LongToDouble => (ValueType::Long, ValueType::Double),
        FloatToInt => (ValueType::Float, ValueType::Integer),
        FloatToLong => (ValueType::Float, ValueType::Long),
        FloatToDouble => (ValueType::Float, ValueType::Double),
        DoubleToInt => (ValueType::Double, ValueType::Integer),
        DoubleToLong => (ValueType::Double, ValueType::Long),
        DoubleToFloat => (ValueType::Double, ValueType::Float),
        IntToByte | IntToChar | IntToShort => (ValueType::Integer, ValueType::Integer),
    }
}

fn element_type(element: ElementType) -> ValueType {
    match element {
        ElementType::Bits32 => ValueType::Bits32,
        ElementType::Bits64 => ValueType::Bits64,
        ElementType::Integer
        | ElementType::Boolean
        | ElementType::Byte
        | ElementType::ByteOrBoolean
        | ElementType::Char
        | ElementType::Short => ValueType::Integer,
        ElementType::Long => ValueType::Long,
        ElementType::Float => ValueType::Float,
        ElementType::Double => ValueType::Double,
        ElementType::Reference => ValueType::Reference(None),
    }
}

fn arithmetic(value_type: &ValueType) -> bool {
    matches!(
        value_type,
        ValueType::Unknown
            | ValueType::Integer
            | ValueType::Long
            | ValueType::Float
            | ValueType::Double
            | ValueType::Bits32
            | ValueType::Zero
            | ValueType::Bits64
    )
}

fn integral(value_type: &ValueType) -> bool {
    matches!(
        value_type,
        ValueType::Unknown
            | ValueType::Integer
            | ValueType::Long
            | ValueType::Bits32
            | ValueType::Zero
            | ValueType::Bits64
    )
}

fn integer_like(value_type: &ValueType) -> bool {
    matches!(
        value_type,
        ValueType::Unknown
            | ValueType::Boolean
            | ValueType::Integer
            | ValueType::Bits32
            | ValueType::Zero
    )
}

fn reference_like(value_type: &ValueType) -> bool {
    matches!(value_type, ValueType::Unknown | ValueType::Zero) || value_type.is_reference()
}

fn issue(issues: &mut Vec<VerificationIssue>, message: impl Into<String>) {
    issues.push(VerificationIssue::new(message));
}
