//! Javap-like text rendering for parsed class files and bytecode.

use std::fmt::{self, Write};

use crate::bytecode::{Instruction, Operand};
use crate::classfile::{
    Attribute, CATCH_ALL_EXCEPTION_INDEX, ClassAccessFlags, ClassFile, CodeAttribute, ConstantPool,
    FieldAccessFlags, FieldInfo, MethodAccessFlags, MethodInfo, RawAttribute,
};
use crate::descriptor::{self, JavaType};
use crate::{Error, Result};

const CONSTANT_POOL_INDEX_WIDTH: usize = 5;
const CONSTANT_TAG_NAME_WIDTH: usize = 18;
const EXCEPTION_RANGE_OFFSET_WIDTH: usize = 4;
const EXCEPTION_HANDLER_TARGET_WIDTH: usize = 6;
const INSTRUCTION_OFFSET_WIDTH: usize = 5;
const OPERAND_COUNT_WIDTH: usize = 3;

macro_rules! flag_names {
    ($flags:expr, $($flag:path => $name:literal),+ $(,)?) => {{
        let mut names = Vec::new();
        $(
            if $flags.contains($flag) {
                names.push($name);
            }
        )+
        names
    }};
}

/// Controls the amount of detail emitted by [`disassemble`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Include a resolved constant-pool listing.
    pub show_constant_pool: bool,
    /// Include uninterpreted attribute names and byte lengths.
    pub show_attributes: bool,
    /// Only show methods with this exact JVM name.
    pub method: Option<String>,
    /// Further restrict methods to this exact JVM descriptor.
    pub descriptor: Option<String>,
}

/// Produces a complete textual disassembly of one parsed class.
///
/// # Errors
///
/// Returns an error if a class reference or descriptor cannot be resolved, a
/// method body is malformed, or the requested method is not present.
pub fn disassemble(class: &ClassFile, options: &Options) -> Result<String> {
    let pool = &class.constant_pool;
    let internal_name = class.class_name()?;
    let class_name = display_class_name(internal_name);
    let mut output = String::new();

    line(&mut output, format_args!("Classfile {internal_name}.class"));
    line(
        &mut output,
        format_args!("  minor version: {}", class.minor_version),
    );
    let release = class
        .java_version()
        .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
    line(
        &mut output,
        format_args!("  major version: {} (Java {release})", class.major_version),
    );
    write_flags(
        &mut output,
        "  ",
        class.access_flags.bits(),
        &class_flag_names(class.access_flags),
    );

    if options.show_constant_pool {
        output.push_str("Constant pool:\n");
        for (index, constant) in pool.iter() {
            line(
                &mut output,
                format_args!(
                    "  #{index:<index_width$} = {tag:<tag_width$} // {description}",
                    index_width = CONSTANT_POOL_INDEX_WIDTH,
                    tag = constant.tag_name(),
                    tag_width = CONSTANT_TAG_NAME_WIDTH,
                    description = pool.describe(index)?,
                ),
            );
        }
        output.push('\n');
    }

    write_declaration(&mut output, class, &class_name)?;
    output.push_str(" {\n");

    for field in &class.fields {
        write_field(&mut output, field, pool, options.show_attributes)?;
    }

    let mut matched_methods = 0_usize;
    for method in &class.methods {
        let name = method.name(pool)?;
        let descriptor = method.descriptor(pool)?;
        if options
            .method
            .as_deref()
            .is_some_and(|filter| name != filter)
            || options
                .descriptor
                .as_deref()
                .is_some_and(|filter| descriptor != filter)
        {
            continue;
        }
        matched_methods += 1;
        write_method(
            &mut output,
            method,
            pool,
            &class_name,
            options.show_attributes,
        )?;
    }

    if let Some(method) = &options.method
        && matched_methods == 0
    {
        return Err(Error::MethodNotFound {
            class: internal_name.to_owned(),
            method: method.clone(),
            descriptor: options
                .descriptor
                .as_ref()
                .map_or_else(String::new, |value| format!(" {value}")),
        });
    }

    output.push_str("}\n");
    if options.show_attributes {
        write_attribute_summary(&mut output, "Class attributes", &class.attributes);
    }
    Ok(output)
}

fn write_declaration(output: &mut String, class: &ClassFile, class_name: &str) -> Result<()> {
    let modifiers = class_modifiers(class.access_flags);
    if !modifiers.is_empty() {
        output.push_str(&modifiers.join(" "));
        output.push(' ');
    }

    let kind = if class.access_flags.contains(ClassAccessFlags::MODULE) {
        "module"
    } else if class.access_flags.contains(ClassAccessFlags::ANNOTATION) {
        "@interface"
    } else if class.access_flags.contains(ClassAccessFlags::INTERFACE) {
        "interface"
    } else if class.access_flags.contains(ClassAccessFlags::ENUM) {
        "enum"
    } else {
        "class"
    };
    write!(output, "{kind} {class_name}").expect("writing to a String cannot fail");

    if let Some(super_name) = class.super_name()?
        && super_name != "java/lang/Object"
        && !class.access_flags.contains(ClassAccessFlags::INTERFACE)
    {
        write!(output, " extends {}", display_class_name(super_name))
            .expect("writing to a String cannot fail");
    }

    if !class.interfaces.is_empty() {
        let relationship = if class.access_flags.contains(ClassAccessFlags::INTERFACE) {
            " extends "
        } else {
            " implements "
        };
        output.push_str(relationship);
        for (position, &index) in class.interfaces.iter().enumerate() {
            if position != 0 {
                output.push_str(", ");
            }
            output.push_str(&display_class_name(class.constant_pool.class_name(index)?));
        }
    }
    Ok(())
}

fn write_field(
    output: &mut String,
    field: &FieldInfo,
    pool: &ConstantPool,
    show_attributes: bool,
) -> Result<()> {
    let name = field.name(pool)?;
    let raw_descriptor = field.descriptor(pool)?;
    let field_type = descriptor::parse_field(raw_descriptor)?;
    let modifiers = field_modifiers(field.access_flags);
    output.push_str("  ");
    if !modifiers.is_empty() {
        output.push_str(&modifiers.join(" "));
        output.push(' ');
    }
    line(output, format_args!("{field_type} {name};"));
    line(output, format_args!("    descriptor: {raw_descriptor}"));
    write_flags(
        output,
        "    ",
        field.access_flags.bits(),
        &field_flag_names(field.access_flags),
    );
    if show_attributes {
        write_attribute_summary(output, "    attributes", &field.attributes);
    }
    output.push('\n');
    Ok(())
}

fn write_method(
    output: &mut String,
    method: &MethodInfo,
    pool: &ConstantPool,
    class_name: &str,
    show_attributes: bool,
) -> Result<()> {
    let name = method.name(pool)?;
    let raw_descriptor = method.descriptor(pool)?;
    let parsed = descriptor::parse_method(raw_descriptor)?;
    let modifiers = method_modifiers(method.access_flags);
    output.push_str("  ");
    if !modifiers.is_empty() {
        output.push_str(&modifiers.join(" "));
        output.push(' ');
    }

    if name == "<clinit>" {
        output.push_str("{};\n");
    } else {
        if name == "<init>" {
            output.push_str(class_name.rsplit('.').next().unwrap_or(class_name));
        } else {
            write!(output, "{} {name}", parsed.return_type)
                .expect("writing to a String cannot fail");
        }
        output.push('(');
        write_parameters(
            output,
            &parsed.parameters,
            method.access_flags.contains(MethodAccessFlags::VARARGS),
        );
        output.push_str(");\n");
    }

    line(output, format_args!("    descriptor: {raw_descriptor}"));
    write_flags(
        output,
        "    ",
        method.access_flags.bits(),
        &method_flag_names(method.access_flags),
    );

    if let Some(code) = method.code() {
        write_code(output, code, pool, show_attributes)?;
    }
    if show_attributes {
        write_method_attribute_summary(output, method);
    }
    output.push('\n');
    Ok(())
}

fn write_code(
    output: &mut String,
    code: &CodeAttribute,
    pool: &ConstantPool,
    show_attributes: bool,
) -> Result<()> {
    let instructions = code.instructions()?;
    line(
        output,
        format_args!(
            "    Code: stack={}, locals={}, bytes={}",
            code.max_stack,
            code.max_locals,
            code.code.len()
        ),
    );
    for instruction in &instructions {
        write_instruction(output, instruction, pool)?;
    }
    write_exception_table(output, code, pool)?;
    if show_attributes {
        write_raw_attribute_summary(output, "      code attributes", &code.attributes);
    }
    Ok(())
}

fn write_exception_table(
    output: &mut String,
    code: &CodeAttribute,
    pool: &ConstantPool,
) -> Result<()> {
    if code.exception_table.is_empty() {
        return Ok(());
    }
    output.push_str("      Exception table:\n");
    output.push_str("         from    to  target  type\n");
    for handler in &code.exception_table {
        let catch_type = if handler.catch_type == CATCH_ALL_EXCEPTION_INDEX {
            "any".to_owned()
        } else {
            format!(
                "#{} // {}",
                handler.catch_type,
                pool.class_name(handler.catch_type)?
            )
        };
        line(
            output,
            format_args!(
                "         {start:>range_width$}  {end:>range_width$}  {target:>target_width$}  {catch_type}",
                start = handler.start_pc,
                end = handler.end_pc,
                target = handler.handler_pc,
                range_width = EXCEPTION_RANGE_OFFSET_WIDTH,
                target_width = EXCEPTION_HANDLER_TARGET_WIDTH,
            ),
        );
    }
    Ok(())
}

fn write_method_attribute_summary(output: &mut String, method: &MethodInfo) {
    let raw = method
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            Attribute::Raw(attribute) => Some(attribute),
            Attribute::Code(_) => None,
        });
    let mut raw = raw.peekable();
    if raw.peek().is_none() {
        return;
    }
    output.push_str("    method attributes:");
    for attribute in raw {
        write!(
            output,
            " {} ({} bytes)",
            attribute.name,
            attribute.info.len()
        )
        .expect("writing to a String cannot fail");
    }
    output.push('\n');
}

fn write_instruction(
    output: &mut String,
    instruction: &Instruction,
    pool: &ConstantPool,
) -> Result<()> {
    write!(
        output,
        "      {offset:>width$}: ",
        offset = instruction.offset,
        width = INSTRUCTION_OFFSET_WIDTH,
    )
    .expect("writing to a String cannot fail");
    if instruction.wide {
        output.push_str("wide ");
    }
    write!(output, "{}", instruction.mnemonic()).expect("writing to a String cannot fail");
    match &instruction.operand {
        Operand::None => {}
        Operand::Byte(value) => write!(output, " {value}").expect("String write"),
        Operand::Short(value) => write!(output, " {value}").expect("String write"),
        Operand::Constant(index) | Operand::InvokeDynamic(index) => {
            write!(
                output,
                " #{index:<width$} // {description}",
                width = CONSTANT_POOL_INDEX_WIDTH,
                description = pool.describe(*index)?,
            )
            .expect("writing to a String cannot fail");
        }
        Operand::Local(index) => write!(output, " {index}").expect("String write"),
        Operand::Increment { index, value } => {
            write!(output, " {index}, {value}").expect("String write");
        }
        Operand::Branch(target) => write!(output, " {target}").expect("String write"),
        Operand::ArrayType(array_type) => {
            write!(output, " {}", array_type.name()).expect("String write");
        }
        Operand::InvokeInterface { index, count } => {
            write!(
                output,
                " #{index}, {count:<width$} // {description}",
                width = OPERAND_COUNT_WIDTH,
                description = pool.describe(*index)?,
            )
            .expect("String write");
        }
        Operand::MultiArray { index, dimensions } => {
            write!(
                output,
                " #{index}, {dimensions:<width$} // {description}",
                width = OPERAND_COUNT_WIDTH,
                description = pool.describe(*index)?,
            )
            .expect("String write");
        }
        Operand::TableSwitch {
            default,
            low,
            targets,
        } => {
            output.push_str(" {\n");
            for (position, target) in targets.iter().enumerate() {
                let key = i64::from(*low) + i64::try_from(position).unwrap_or(i64::MAX);
                line(output, format_args!("                 {key}: {target}"));
            }
            line(output, format_args!("           default: {default}"));
            output.push_str("              }");
        }
        Operand::LookupSwitch { default, pairs } => {
            output.push_str(" {\n");
            for (key, target) in pairs {
                line(output, format_args!("                 {key}: {target}"));
            }
            line(output, format_args!("           default: {default}"));
            output.push_str("              }");
        }
    }
    output.push('\n');
    Ok(())
}

fn write_parameters(output: &mut String, parameters: &[JavaType], varargs: bool) {
    for (index, parameter) in parameters.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        if varargs
            && index + 1 == parameters.len()
            && let JavaType::Array(element) = parameter
        {
            write!(output, "{element}...").expect("writing to a String cannot fail");
            continue;
        }
        write!(output, "{parameter}").expect("writing to a String cannot fail");
    }
}

fn write_attribute_summary(output: &mut String, label: &str, attributes: &[Attribute]) {
    if attributes.is_empty() {
        return;
    }
    write!(output, "{label}:").expect("writing to a String cannot fail");
    for attribute in attributes {
        let length = match attribute {
            Attribute::Code(code) => code.code.len(),
            Attribute::Raw(attribute) => attribute.info.len(),
        };
        write!(output, " {} ({length} bytes)", attribute.name())
            .expect("writing to a String cannot fail");
    }
    output.push('\n');
}

fn write_raw_attribute_summary(output: &mut String, label: &str, attributes: &[RawAttribute]) {
    if attributes.is_empty() {
        return;
    }
    write!(output, "{label}:").expect("writing to a String cannot fail");
    for attribute in attributes {
        write!(
            output,
            " {} ({} bytes)",
            attribute.name,
            attribute.info.len()
        )
        .expect("writing to a String cannot fail");
    }
    output.push('\n');
}

fn line(output: &mut String, arguments: fmt::Arguments<'_>) {
    output
        .write_fmt(arguments)
        .expect("writing to a String cannot fail");
    output.push('\n');
}

fn write_flags(output: &mut String, indentation: &str, flags: u16, names: &[&str]) {
    write!(output, "{indentation}flags: 0x{flags:04x}").expect("writing to a String cannot fail");
    if !names.is_empty() {
        output.push(' ');
        output.push_str(&names.join(", "));
    }
    output.push('\n');
}

fn display_class_name(internal_name: &str) -> String {
    internal_name.replace('/', ".")
}

fn class_modifiers(flags: ClassAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        ClassAccessFlags::PUBLIC => "public",
        ClassAccessFlags::FINAL => "final",
        ClassAccessFlags::ABSTRACT => "abstract",
        ClassAccessFlags::SYNTHETIC => "synthetic",
    )
}

fn field_modifiers(flags: FieldAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        FieldAccessFlags::PUBLIC => "public",
        FieldAccessFlags::PRIVATE => "private",
        FieldAccessFlags::PROTECTED => "protected",
        FieldAccessFlags::STATIC => "static",
        FieldAccessFlags::FINAL => "final",
        FieldAccessFlags::VOLATILE => "volatile",
        FieldAccessFlags::TRANSIENT => "transient",
        FieldAccessFlags::SYNTHETIC => "synthetic",
    )
}

fn method_modifiers(flags: MethodAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        MethodAccessFlags::PUBLIC => "public",
        MethodAccessFlags::PRIVATE => "private",
        MethodAccessFlags::PROTECTED => "protected",
        MethodAccessFlags::STATIC => "static",
        MethodAccessFlags::FINAL => "final",
        MethodAccessFlags::SYNCHRONIZED => "synchronized",
        MethodAccessFlags::NATIVE => "native",
        MethodAccessFlags::ABSTRACT => "abstract",
        MethodAccessFlags::STRICT => "strictfp",
        MethodAccessFlags::SYNTHETIC => "synthetic",
    )
}

fn class_flag_names(flags: ClassAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        ClassAccessFlags::PUBLIC => "ACC_PUBLIC",
        ClassAccessFlags::FINAL => "ACC_FINAL",
        ClassAccessFlags::SUPER => "ACC_SUPER",
        ClassAccessFlags::INTERFACE => "ACC_INTERFACE",
        ClassAccessFlags::ABSTRACT => "ACC_ABSTRACT",
        ClassAccessFlags::SYNTHETIC => "ACC_SYNTHETIC",
        ClassAccessFlags::ANNOTATION => "ACC_ANNOTATION",
        ClassAccessFlags::ENUM => "ACC_ENUM",
        ClassAccessFlags::MODULE => "ACC_MODULE",
    )
}

fn field_flag_names(flags: FieldAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        FieldAccessFlags::PUBLIC => "ACC_PUBLIC",
        FieldAccessFlags::PRIVATE => "ACC_PRIVATE",
        FieldAccessFlags::PROTECTED => "ACC_PROTECTED",
        FieldAccessFlags::STATIC => "ACC_STATIC",
        FieldAccessFlags::FINAL => "ACC_FINAL",
        FieldAccessFlags::VOLATILE => "ACC_VOLATILE",
        FieldAccessFlags::TRANSIENT => "ACC_TRANSIENT",
        FieldAccessFlags::SYNTHETIC => "ACC_SYNTHETIC",
        FieldAccessFlags::ENUM => "ACC_ENUM",
    )
}

fn method_flag_names(flags: MethodAccessFlags) -> Vec<&'static str> {
    flag_names!(
        flags,
        MethodAccessFlags::PUBLIC => "ACC_PUBLIC",
        MethodAccessFlags::PRIVATE => "ACC_PRIVATE",
        MethodAccessFlags::PROTECTED => "ACC_PROTECTED",
        MethodAccessFlags::STATIC => "ACC_STATIC",
        MethodAccessFlags::FINAL => "ACC_FINAL",
        MethodAccessFlags::SYNCHRONIZED => "ACC_SYNCHRONIZED",
        MethodAccessFlags::BRIDGE => "ACC_BRIDGE",
        MethodAccessFlags::VARARGS => "ACC_VARARGS",
        MethodAccessFlags::NATIVE => "ACC_NATIVE",
        MethodAccessFlags::ABSTRACT => "ACC_ABSTRACT",
        MethodAccessFlags::STRICT => "ACC_STRICT",
        MethodAccessFlags::SYNTHETIC => "ACC_SYNTHETIC",
    )
}
