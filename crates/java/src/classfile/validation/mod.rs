//! Full structural validation for parsed or programmatically built classes.

mod constants;

use std::collections::HashSet;

use crate::bytecode::{Instruction, Opcode, Operand};
use crate::descriptor::{self, JavaType};
use crate::{Error, Result};

use self::constants::{
    bootstrap_method_count, dynamic_has_category_two_descriptor, validate_constant_pool,
    validate_dynamic_bootstraps,
};
use super::attribute::validate_known_model;
use super::{
    Attribute, AttributeLocation, ClassAccessFlags, ClassFile, CodeAttribute, Constant,
    ConstantPool, FieldAccessFlags, KnownAttribute, MethodAccessFlags, StackMapFrame,
    TypeAnnotationTarget, VerificationType,
};

/// Oldest supported class-file major version (Java 1.1).
pub const MIN_SUPPORTED_CLASS_MAJOR: u16 = 45;
/// Newest supported class-file major version (Java SE 26).
pub const MAX_SUPPORTED_CLASS_MAJOR: u16 = 70;

const PREVIEW_MINOR_VERSION: u16 = u16::MAX;
const FIRST_PREVIEW_MAJOR_VERSION: u16 = 56;

/// Deterministic aggregate counts produced by class validation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassValidationReport {
    /// Number of usable constant-pool entries.
    pub constants: usize,
    /// Number of declared fields.
    pub fields: usize,
    /// Number of declared methods.
    pub methods: usize,
    /// Number of methods containing code.
    pub code_methods: usize,
    /// Number of decoded JVM instructions.
    pub instructions: usize,
    /// Number of standard and custom attributes, including nested attributes.
    pub attributes: usize,
}

pub(crate) fn validate_class(class: &ClassFile) -> Result<ClassValidationReport> {
    validate_version(class)?;
    class.constant_pool.validate()?;
    validate_constant_pool(class)?;
    validate_class_header(class)?;
    let bootstrap_count = bootstrap_method_count(class)?;
    validate_dynamic_bootstraps(class, bootstrap_count)?;

    let mut report = ClassValidationReport {
        constants: class.constant_pool.iter().count(),
        fields: class.fields.len(),
        methods: class.methods.len(),
        ..ClassValidationReport::default()
    };
    validate_attributes(
        &class.attributes,
        AttributeLocation::Class,
        class,
        None,
        &mut report,
    )?;
    validate_fields(class, &mut report)?;
    validate_methods(class, &mut report)?;
    validate_module_shape(class)?;
    Ok(report)
}

fn validate_version(class: &ClassFile) -> Result<()> {
    if !(MIN_SUPPORTED_CLASS_MAJOR..=MAX_SUPPORTED_CLASS_MAJOR).contains(&class.major_version) {
        return Err(invalid(format!(
            "unsupported class-file version {}.{}; supported majors are {MIN_SUPPORTED_CLASS_MAJOR} through {MAX_SUPPORTED_CLASS_MAJOR}",
            class.major_version, class.minor_version
        )));
    }
    if class.major_version >= FIRST_PREVIEW_MAJOR_VERSION
        && !matches!(class.minor_version, 0 | PREVIEW_MINOR_VERSION)
    {
        return Err(invalid(format!(
            "class-file major {} requires minor version 0 or 65535",
            class.major_version
        )));
    }
    Ok(())
}

fn validate_class_header(class: &ClassFile) -> Result<()> {
    let pool = &class.constant_pool;
    let name = pool.class_name(class.this_class)?;
    validate_internal_or_array_name(name, false)?;
    if class.super_class == 0 {
        if name != "java/lang/Object" && !class.access_flags.contains(ClassAccessFlags::MODULE) {
            return Err(invalid(
                "only java/lang/Object or module-info may have no superclass",
            ));
        }
    } else {
        validate_internal_or_array_name(pool.class_name(class.super_class)?, false)?;
    }
    let mut interfaces = HashSet::new();
    for &index in &class.interfaces {
        let interface = pool.class_name(index)?;
        validate_internal_or_array_name(interface, false)?;
        if !interfaces.insert(interface) {
            return Err(invalid(format!("duplicate direct interface `{interface}`")));
        }
    }
    let flags = class.access_flags;
    if flags.contains(ClassAccessFlags::INTERFACE) {
        if !flags.contains(ClassAccessFlags::ABSTRACT)
            || flags.contains(ClassAccessFlags::FINAL)
            || flags.contains(ClassAccessFlags::SUPER)
            || flags.contains(ClassAccessFlags::ENUM)
            || flags.contains(ClassAccessFlags::MODULE)
        {
            return Err(invalid("invalid interface access-flag combination"));
        }
    } else if flags.contains(ClassAccessFlags::ANNOTATION) {
        return Err(invalid("ACC_ANNOTATION requires ACC_INTERFACE"));
    }
    if flags.contains(ClassAccessFlags::FINAL) && flags.contains(ClassAccessFlags::ABSTRACT) {
        return Err(invalid("a class cannot be both final and abstract"));
    }
    Ok(())
}

fn validate_fields(class: &ClassFile, report: &mut ClassValidationReport) -> Result<()> {
    let mut declarations = HashSet::new();
    for field in &class.fields {
        let name = field.name(&class.constant_pool)?;
        validate_unqualified_name(name, false)?;
        let descriptor = field.descriptor(&class.constant_pool)?;
        descriptor::parse_field(descriptor)?;
        if !declarations.insert((name, descriptor)) {
            return Err(invalid(format!("duplicate field `{name}:{descriptor}`")));
        }
        validate_visibility(field.access_flags.bits(), "field", name)?;
        if field.access_flags.contains(FieldAccessFlags::FINAL)
            && field.access_flags.contains(FieldAccessFlags::VOLATILE)
        {
            return Err(invalid(format!(
                "field `{name}` cannot be final and volatile"
            )));
        }
        validate_attributes(
            &field.attributes,
            AttributeLocation::Field,
            class,
            None,
            report,
        )?;
    }
    Ok(())
}

fn validate_methods(class: &ClassFile, report: &mut ClassValidationReport) -> Result<()> {
    let mut declarations = HashSet::new();
    let owner = class.class_name()?.to_owned();
    for method in &class.methods {
        let unresolved_name = format!("#{}", method.name_index);
        let name = method
            .name(&class.constant_pool)
            .map_err(|error| error.in_class_method(&owner, &unresolved_name, String::new()))?;
        let unresolved_descriptor = format!("#{}", method.descriptor_index);
        let descriptor_text = method
            .descriptor(&class.constant_pool)
            .map_err(|error| error.in_class_method(&owner, name, &unresolved_descriptor))?;
        let contextualize = |error: Error| error.in_class_method(&owner, name, descriptor_text);
        validate_unqualified_name(name, true).map_err(&contextualize)?;
        let descriptor = descriptor::parse_method(descriptor_text).map_err(&contextualize)?;
        if !declarations.insert((name, descriptor_text)) {
            return Err(contextualize(invalid("duplicate method declaration")));
        }
        validate_visibility(method.access_flags.bits(), "method", name).map_err(&contextualize)?;
        validate_method_flags(method.access_flags, name).map_err(&contextualize)?;
        if name == "<init>" && !matches!(descriptor.return_type, descriptor::ReturnType::Void) {
            return Err(contextualize(invalid(
                "constructor descriptor must return void",
            )));
        }
        if name == "<clinit>" && descriptor_text != "()V" {
            return Err(contextualize(invalid(
                "class initializer descriptor must be ()V",
            )));
        }
        let code_count = method
            .attributes
            .iter()
            .filter(|attribute| matches!(attribute, Attribute::Code(_)))
            .count();
        let body_forbidden = method.access_flags.contains(MethodAccessFlags::ABSTRACT)
            || method.access_flags.contains(MethodAccessFlags::NATIVE);
        if (body_forbidden && code_count != 0) || (!body_forbidden && code_count != 1) {
            return Err(contextualize(invalid(format!(
                "method has {code_count} Code attributes"
            ))));
        }
        validate_attributes(
            &method.attributes,
            AttributeLocation::Method,
            class,
            None,
            report,
        )
        .map_err(contextualize)?;
    }
    Ok(())
}

fn validate_method_flags(flags: MethodAccessFlags, name: &str) -> Result<()> {
    if flags.contains(MethodAccessFlags::ABSTRACT)
        && (flags.contains(MethodAccessFlags::PRIVATE)
            || flags.contains(MethodAccessFlags::STATIC)
            || flags.contains(MethodAccessFlags::FINAL)
            || flags.contains(MethodAccessFlags::SYNCHRONIZED)
            || flags.contains(MethodAccessFlags::NATIVE)
            || flags.contains(MethodAccessFlags::STRICT))
    {
        return Err(invalid(format!(
            "abstract method `{name}` has incompatible access flags"
        )));
    }
    Ok(())
}

fn validate_attributes(
    attributes: &[Attribute],
    location: AttributeLocation,
    class: &ClassFile,
    code: Option<&CodeAttribute>,
    report: &mut ClassValidationReport,
) -> Result<()> {
    report.attributes += attributes.len();
    let mut singleton_names = HashSet::new();
    for attribute in attributes {
        let name = attribute.name();
        let indexed_name = class.constant_pool.utf8(attribute.name_index())?;
        if indexed_name != name {
            return Err(invalid(format!(
                "attribute name index #{} resolves to `{indexed_name}`, expected `{name}`",
                attribute.name_index()
            )));
        }
        if is_singleton_attribute(name) && !singleton_names.insert(name) {
            return Err(invalid(format!(
                "duplicate `{name}` attribute at {location:?} location"
            )));
        }
        match attribute {
            Attribute::Code(body) => {
                if location != AttributeLocation::Method {
                    return Err(invalid("Code attribute is not attached to a method"));
                }
                validate_code(body, class, report)?;
            }
            Attribute::Known(known) => {
                if !known.is_valid_at(location) {
                    return Err(invalid(format!(
                        "{} attribute is invalid at {location:?} location",
                        known.name()
                    )));
                }
                validate_known_version(known, class.major_version)?;
                validate_known_model(known, &class.constant_pool)?;
                validate_type_targets(known, location)?;
                if let KnownAttribute::Record(record) = known {
                    for component in &record.components {
                        validate_unqualified_name(
                            class.constant_pool.utf8(component.name_index)?,
                            false,
                        )?;
                        descriptor::parse_field(
                            class.constant_pool.utf8(component.descriptor_index)?,
                        )?;
                        validate_attributes(
                            &component.attributes,
                            AttributeLocation::RecordComponent,
                            class,
                            None,
                            report,
                        )?;
                    }
                }
                if let Some(code) = code {
                    validate_code_metadata(known, code)?;
                }
            }
            Attribute::Raw(raw) => {
                if is_standard_attribute_name(&raw.name) {
                    return Err(invalid(format!(
                        "standard attribute `{}` must use its typed representation",
                        raw.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_code(
    code: &CodeAttribute,
    class: &ClassFile,
    report: &mut ClassValidationReport,
) -> Result<()> {
    let instructions = bytecode_instructions(code)?;
    for instruction in &instructions {
        validate_instruction(instruction, code, class)?;
    }
    report.code_methods += 1;
    report.instructions += instructions.len();
    validate_attributes(
        &code.attributes,
        AttributeLocation::Code,
        class,
        Some(code),
        report,
    )
}

fn bytecode_instructions(code: &CodeAttribute) -> Result<Vec<Instruction>> {
    crate::bytecode::decode_code(code)
}

fn validate_instruction(
    instruction: &Instruction,
    code: &CodeAttribute,
    class: &ClassFile,
) -> Result<()> {
    let pool = &class.constant_pool;
    match (&instruction.operand, instruction.opcode) {
        (Operand::Constant(index), Opcode::Ldc | Opcode::LdcW) => {
            expect_tag(pool, *index, "category-1 loadable constant", |constant| {
                matches!(
                    constant,
                    Constant::Integer(_)
                        | Constant::Float(_)
                        | Constant::String { .. }
                        | Constant::Class { .. }
                        | Constant::MethodHandle { .. }
                        | Constant::MethodType { .. }
                        | Constant::Dynamic { .. }
                )
            })?;
        }
        (Operand::Constant(index), Opcode::Ldc2W) => {
            expect_tag(pool, *index, "category-2 loadable constant", |constant| {
                matches!(constant, Constant::Long(_) | Constant::Double(_))
                    || dynamic_has_category_two_descriptor(pool, constant)
            })?;
        }
        (
            Operand::Constant(index),
            Opcode::GetStatic | Opcode::PutStatic | Opcode::GetField | Opcode::PutField,
        ) => expect_tag(pool, *index, "Fieldref", |constant| {
            matches!(constant, Constant::FieldRef { .. })
        })?,
        (Operand::Constant(index), Opcode::InvokeVirtual) => {
            expect_tag(pool, *index, "Methodref", |constant| {
                matches!(constant, Constant::MethodRef { .. })
            })?;
        }
        (Operand::Constant(index), Opcode::InvokeSpecial | Opcode::InvokeStatic) => {
            expect_tag(pool, *index, "method reference", |constant| {
                matches!(
                    constant,
                    Constant::MethodRef { .. } | Constant::InterfaceMethodRef { .. }
                )
            })?;
        }
        (Operand::InvokeInterface { index, count }, Opcode::InvokeInterface) => {
            let descriptor = referenced_method_descriptor(pool, *index, true)?;
            let expected = parameter_slots(&descriptor)?
                .checked_add(1)
                .ok_or_else(|| invalid("invokeinterface argument count overflow"))?;
            if usize::from(*count) != expected {
                return Err(invalid(format!(
                    "invokeinterface at {} records count {count}, expected {expected}",
                    instruction.offset
                )));
            }
        }
        (Operand::InvokeDynamic(index), Opcode::InvokeDynamic) => {
            expect_tag(pool, *index, "InvokeDynamic", |constant| {
                matches!(constant, Constant::InvokeDynamic { .. })
            })?;
        }
        (
            Operand::Constant(index),
            Opcode::New | Opcode::ANewArray | Opcode::CheckCast | Opcode::InstanceOf,
        ) => {
            expect_class(pool, *index)?;
            if instruction.opcode == Opcode::New && pool.class_name(*index)?.starts_with('[') {
                return Err(invalid("new instruction references an array class"));
            }
        }
        (Operand::MultiArray { index, dimensions }, Opcode::MultiANewArray) => {
            let name = pool.class_name(*index)?;
            let available = name.bytes().take_while(|byte| *byte == b'[').count();
            if available == 0 || usize::from(*dimensions) > available {
                return Err(invalid(format!(
                    "multianewarray dimensions {dimensions} exceed `{name}`"
                )));
            }
        }
        (Operand::Local(index) | Operand::Increment { index, .. }, _)
            if *index >= code.max_locals =>
        {
            return Err(invalid(format!(
                "local index {index} at bytecode offset {} exceeds max_locals {}",
                instruction.offset, code.max_locals
            )));
        }
        _ => {}
    }
    Ok(())
}

fn validate_code_metadata(attribute: &KnownAttribute, code: &CodeAttribute) -> Result<()> {
    let instructions = crate::bytecode::decode(&code.code)?;
    let boundaries: HashSet<u16> = instructions
        .iter()
        .filter_map(|instruction| u16::try_from(instruction.offset).ok())
        .chain(u16::try_from(code.code.len()).ok())
        .collect();
    match attribute {
        KnownAttribute::LineNumberTable(table) => {
            for line in &table.lines {
                require_instruction_offset(line.start_pc, &boundaries, code, "line number")?;
            }
        }
        KnownAttribute::LocalVariableTable(table) => {
            for variable in &table.variables {
                validate_range(variable.start_pc, variable.length, &boundaries, code)?;
                if variable.slot >= code.max_locals {
                    return Err(invalid("local-variable table slot exceeds max_locals"));
                }
            }
        }
        KnownAttribute::LocalVariableTypeTable(table) => {
            for variable in &table.variables {
                validate_range(variable.start_pc, variable.length, &boundaries, code)?;
                if variable.slot >= code.max_locals {
                    return Err(invalid("local-variable type-table slot exceeds max_locals"));
                }
            }
        }
        KnownAttribute::StackMapTable(table) => {
            validate_stack_maps(&table.frames, &instructions, &boundaries, code)?;
        }
        KnownAttribute::RuntimeVisibleTypeAnnotations(table)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(table) => {
            for annotation in &table.annotations {
                validate_code_type_target(&annotation.target, &boundaries, code)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_stack_maps(
    frames: &[StackMapFrame],
    instructions: &[Instruction],
    boundaries: &HashSet<u16>,
    code: &CodeAttribute,
) -> Result<()> {
    let mut previous: Option<u32> = None;
    for frame in frames {
        let absolute = match previous {
            None => u32::from(frame.offset_delta()),
            Some(previous) => previous + u32::from(frame.offset_delta()) + 1,
        };
        let offset = u16::try_from(absolute)
            .map_err(|_| invalid("stack-map frame bytecode offset overflows u16"))?;
        require_instruction_offset(offset, boundaries, code, "stack-map frame")?;
        validate_verification_offsets(frame, instructions)?;
        previous = Some(absolute);
    }
    Ok(())
}

fn validate_verification_offsets(
    frame: &StackMapFrame,
    instructions: &[Instruction],
) -> Result<()> {
    let mut values = Vec::new();
    match frame {
        StackMapFrame::SameLocalsOneStack { stack, .. }
        | StackMapFrame::SameLocalsOneStackExtended { stack, .. } => values.push(*stack),
        StackMapFrame::Append { locals, .. } => values.extend(locals.iter().copied()),
        StackMapFrame::Full { locals, stack, .. } => {
            values.extend(locals.iter().chain(stack).copied());
        }
        StackMapFrame::Same { .. }
        | StackMapFrame::Chop { .. }
        | StackMapFrame::SameExtended { .. } => {}
    }
    for value in values {
        if let VerificationType::Uninitialized(offset) = value
            && !instructions.iter().any(|instruction| {
                instruction.offset == usize::from(offset) && instruction.opcode == Opcode::New
            })
        {
            return Err(invalid(format!(
                "uninitialized verification type does not reference `new` at {offset}"
            )));
        }
    }
    Ok(())
}

fn validate_code_type_target(
    target: &TypeAnnotationTarget,
    boundaries: &HashSet<u16>,
    code: &CodeAttribute,
) -> Result<()> {
    match target {
        TypeAnnotationTarget::LocalVariable(targets)
        | TypeAnnotationTarget::ResourceVariable(targets) => {
            for target in targets {
                validate_range(target.start_pc, target.length, boundaries, code)?;
                if target.index >= code.max_locals {
                    return Err(invalid("type-annotation local slot exceeds max_locals"));
                }
            }
        }
        TypeAnnotationTarget::ExceptionParameter(index) => {
            if usize::from(*index) >= code.exception_table.len() {
                return Err(invalid(
                    "type annotation exception-table index is out of range",
                ));
            }
        }
        TypeAnnotationTarget::InstanceOf(offset)
        | TypeAnnotationTarget::New(offset)
        | TypeAnnotationTarget::ConstructorReference(offset)
        | TypeAnnotationTarget::MethodReference(offset)
        | TypeAnnotationTarget::Cast { offset, .. }
        | TypeAnnotationTarget::ConstructorInvocationTypeArgument { offset, .. }
        | TypeAnnotationTarget::MethodInvocationTypeArgument { offset, .. }
        | TypeAnnotationTarget::ConstructorReferenceTypeArgument { offset, .. }
        | TypeAnnotationTarget::MethodReferenceTypeArgument { offset, .. } => {
            require_instruction_offset(*offset, boundaries, code, "type annotation")?;
        }
        _ => {
            return Err(invalid(
                "non-code type-annotation target inside Code attribute",
            ));
        }
    }
    Ok(())
}

fn validate_range(
    start: u16,
    length: u16,
    boundaries: &HashSet<u16>,
    code: &CodeAttribute,
) -> Result<()> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| invalid("bytecode metadata range overflows u16"))?;
    require_instruction_offset(start, boundaries, code, "metadata range start")?;
    if !boundaries.contains(&end) || usize::from(end) > code.code.len() {
        return Err(invalid(format!(
            "bytecode metadata range end {end} is not an instruction boundary"
        )));
    }
    Ok(())
}

fn require_instruction_offset(
    offset: u16,
    boundaries: &HashSet<u16>,
    code: &CodeAttribute,
    label: &str,
) -> Result<()> {
    if usize::from(offset) >= code.code.len() || !boundaries.contains(&offset) {
        Err(invalid(format!(
            "{label} offset {offset} is not an instruction boundary"
        )))
    } else {
        Ok(())
    }
}

fn validate_module_shape(class: &ClassFile) -> Result<()> {
    if !class.access_flags.contains(ClassAccessFlags::MODULE) {
        return Ok(());
    }
    if class.major_version < 53
        || class.access_flags.bits() != ClassAccessFlags::MODULE.bits()
        || class.constant_pool.class_name(class.this_class)? != "module-info"
        || class.super_class != 0
        || !class.interfaces.is_empty()
        || !class.fields.is_empty()
        || !class.methods.is_empty()
    {
        return Err(invalid("invalid module-info class shape"));
    }
    let module_count = class
        .attributes
        .iter()
        .filter(|attribute| matches!(attribute, Attribute::Known(KnownAttribute::Module(_))))
        .count();
    if module_count != 1 {
        return Err(invalid(format!(
            "module-info class has {module_count} Module attributes"
        )));
    }
    Ok(())
}

fn validate_known_version(attribute: &KnownAttribute, major: u16) -> Result<()> {
    let required = match attribute {
        KnownAttribute::StackMapTable(_) => 50,
        KnownAttribute::BootstrapMethods(_) => 51,
        KnownAttribute::RuntimeVisibleTypeAnnotations(_)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(_)
        | KnownAttribute::MethodParameters(_) => 52,
        KnownAttribute::Module(_)
        | KnownAttribute::ModulePackages(_)
        | KnownAttribute::ModuleMainClass(_) => 53,
        KnownAttribute::NestHost(_) | KnownAttribute::NestMembers(_) => 55,
        KnownAttribute::Record(_) => 60,
        KnownAttribute::PermittedSubclasses(_) => 61,
        _ => return Ok(()),
    };
    if major < required {
        Err(invalid(format!(
            "{} attribute requires class-file major {required}",
            attribute.name()
        )))
    } else {
        Ok(())
    }
}

fn validate_type_targets(attribute: &KnownAttribute, location: AttributeLocation) -> Result<()> {
    let annotations = match attribute {
        KnownAttribute::RuntimeVisibleTypeAnnotations(attribute)
        | KnownAttribute::RuntimeInvisibleTypeAnnotations(attribute) => &attribute.annotations,
        _ => return Ok(()),
    };
    for annotation in annotations {
        let valid = match location {
            AttributeLocation::Class => matches!(
                annotation.target,
                TypeAnnotationTarget::ClassTypeParameter(_)
                    | TypeAnnotationTarget::ClassExtends(_)
                    | TypeAnnotationTarget::ClassTypeParameterBound { .. }
            ),
            AttributeLocation::Field | AttributeLocation::RecordComponent => {
                matches!(annotation.target, TypeAnnotationTarget::Field)
            }
            AttributeLocation::Method => matches!(
                annotation.target,
                TypeAnnotationTarget::MethodTypeParameter(_)
                    | TypeAnnotationTarget::MethodTypeParameterBound { .. }
                    | TypeAnnotationTarget::MethodReturn
                    | TypeAnnotationTarget::MethodReceiver
                    | TypeAnnotationTarget::MethodFormalParameter(_)
                    | TypeAnnotationTarget::Throws(_)
            ),
            AttributeLocation::Code => matches!(
                annotation.target,
                TypeAnnotationTarget::LocalVariable(_)
                    | TypeAnnotationTarget::ResourceVariable(_)
                    | TypeAnnotationTarget::ExceptionParameter(_)
                    | TypeAnnotationTarget::InstanceOf(_)
                    | TypeAnnotationTarget::New(_)
                    | TypeAnnotationTarget::ConstructorReference(_)
                    | TypeAnnotationTarget::MethodReference(_)
                    | TypeAnnotationTarget::Cast { .. }
                    | TypeAnnotationTarget::ConstructorInvocationTypeArgument { .. }
                    | TypeAnnotationTarget::MethodInvocationTypeArgument { .. }
                    | TypeAnnotationTarget::ConstructorReferenceTypeArgument { .. }
                    | TypeAnnotationTarget::MethodReferenceTypeArgument { .. }
            ),
        };
        if !valid {
            return Err(invalid(format!(
                "type-annotation target is invalid at {location:?} location"
            )));
        }
    }
    Ok(())
}

fn is_singleton_attribute(name: &str) -> bool {
    is_standard_attribute_name(name)
        && !matches!(
            name,
            "LineNumberTable" | "LocalVariableTable" | "LocalVariableTypeTable"
        )
}

fn is_standard_attribute_name(name: &str) -> bool {
    matches!(
        name,
        "ConstantValue"
            | "Code"
            | "StackMapTable"
            | "Exceptions"
            | "InnerClasses"
            | "EnclosingMethod"
            | "Synthetic"
            | "Signature"
            | "SourceFile"
            | "SourceDebugExtension"
            | "LineNumberTable"
            | "LocalVariableTable"
            | "LocalVariableTypeTable"
            | "Deprecated"
            | "RuntimeVisibleAnnotations"
            | "RuntimeInvisibleAnnotations"
            | "RuntimeVisibleParameterAnnotations"
            | "RuntimeInvisibleParameterAnnotations"
            | "RuntimeVisibleTypeAnnotations"
            | "RuntimeInvisibleTypeAnnotations"
            | "AnnotationDefault"
            | "BootstrapMethods"
            | "MethodParameters"
            | "Module"
            | "ModulePackages"
            | "ModuleMainClass"
            | "NestHost"
            | "NestMembers"
            | "Record"
            | "PermittedSubclasses"
    )
}

fn validate_visibility(bits: u16, kind: &str, name: &str) -> Result<()> {
    let visibility = usize::from(bits & 0x0001 != 0)
        + usize::from(bits & 0x0002 != 0)
        + usize::from(bits & 0x0004 != 0);
    if visibility > 1 {
        Err(invalid(format!(
            "{kind} `{name}` has conflicting visibility flags"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn validate_unqualified_name(name: &str, method: bool) -> Result<()> {
    let is_invalid = name.is_empty()
        || name
            .chars()
            .any(|character| matches!(character, '.' | ';' | '[' | '/'))
        || (method && name != "<init>" && name != "<clinit>" && name.contains(['<', '>']));
    if is_invalid {
        Err(invalid(format!("invalid JVM unqualified name `{name}`")))
    } else {
        Ok(())
    }
}

pub(super) fn validate_internal_or_array_name(name: &str, array_allowed: bool) -> Result<()> {
    if name.starts_with('[') {
        if !array_allowed || !matches!(descriptor::parse_field(name)?, JavaType::Array(_)) {
            return Err(invalid(format!("invalid class constant name `{name}`")));
        }
        return Ok(());
    }
    if name.is_empty()
        || name.starts_with('/')
        || name.ends_with('/')
        || name.split('/').any(str::is_empty)
        || name
            .chars()
            .any(|character| matches!(character, '.' | ';' | '['))
    {
        return Err(invalid(format!("invalid internal class name `{name}`")));
    }
    Ok(())
}

fn referenced_method_descriptor(
    pool: &ConstantPool,
    index: u16,
    interface: bool,
) -> Result<descriptor::MethodDescriptor> {
    let name_and_type = match pool.get(index)? {
        Constant::InterfaceMethodRef {
            name_and_type_index,
            ..
        } if interface => *name_and_type_index,
        constant => {
            return Err(invalid(format!(
                "constant #{index} is {}, expected InterfaceMethodref",
                constant.tag_name()
            )));
        }
    };
    let (_, descriptor) = pool.name_and_type(name_and_type)?;
    descriptor::parse_method(descriptor)
}

fn parameter_slots(descriptor: &descriptor::MethodDescriptor) -> Result<usize> {
    let mut slots = 0_usize;
    for parameter in &descriptor.parameters {
        slots = slots
            .checked_add(usize::from(matches!(parameter, JavaType::Long | JavaType::Double)) + 1)
            .ok_or_else(|| invalid("method parameter slot count overflow"))?;
    }
    Ok(slots)
}

fn expect_class(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Class", |constant| {
        matches!(constant, Constant::Class { .. })
    })
}

fn expect_tag(
    pool: &ConstantPool,
    index: u16,
    expected: &str,
    predicate: impl Fn(&Constant) -> bool,
) -> Result<()> {
    let constant = pool.get(index)?;
    if predicate(constant) {
        Ok(())
    } else {
        Err(invalid(format!(
            "constant #{index} is {}, expected {expected}",
            constant.tag_name()
        )))
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::invalid_class(0, message)
}
