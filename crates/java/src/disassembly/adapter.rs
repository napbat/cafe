//! Class and method lowering into shared disassembly IR.

use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, Disassembly,
    DisassemblySource, ExceptionHandler as SharedExceptionHandler, Function, FunctionBody,
    FunctionSymbol, RawAccessFlags,
};

use super::instruction::lower_instruction;
use crate::classfile::{CATCH_ALL_EXCEPTION_INDEX, ClassFile, CodeAttribute, MethodInfo};
use crate::{Error, Result};

/// Lowers a parsed JVM class into the shared cross-format disassembly model.
///
/// # Errors
///
/// Returns an error when a method name, descriptor, constant reference,
/// instruction, or exception catch type cannot be decoded or resolved.
pub fn lower_class(class: &ClassFile) -> Result<Disassembly> {
    let owner = class.class_name()?.to_owned();
    let functions = class
        .methods
        .iter()
        .map(|method| {
            let name = method
                .name(&class.constant_pool)
                .unwrap_or("<invalid-name>");
            let descriptor = method
                .descriptor(&class.constant_pool)
                .unwrap_or("<invalid-descriptor>");
            lower_method(class, method, &owner)
                .map_err(|error| error.in_class_method(&owner, name, descriptor))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Disassembly {
        format: BinaryFormat::JavaClass,
        name: owner,
        functions,
    })
}

impl DisassemblySource for ClassFile {
    type Error = Error;

    fn disassemble(&self) -> Result<Disassembly> {
        lower_class(self)
    }
}

/// Lowers one JVM method into the neutral disassembly model.
///
/// # Errors
///
/// Returns an error for unresolved method metadata, malformed bytecode, or an
/// invalid native control-flow target.
pub fn lower_method(class: &ClassFile, method: &MethodInfo, owner: &str) -> Result<Function> {
    let pool = &class.constant_pool;
    let body = method
        .code()
        .map(|code| lower_body(code, class))
        .transpose()?;
    Ok(Function {
        symbol: FunctionSymbol {
            owner: owner.to_owned(),
            name: method.name(pool)?.to_owned(),
            signature: method.descriptor(pool)?.to_owned(),
        },
        access_flags: RawAccessFlags::new(u32::from(method.access_flags.bits())),
        body,
    })
}

fn lower_body(code: &CodeAttribute, class: &ClassFile) -> Result<FunctionBody> {
    let instructions = code
        .instructions()?
        .iter()
        .map(|instruction| lower_instruction(instruction, &class.constant_pool))
        .collect::<Result<Vec<_>>>()?;
    let exception_handlers = code
        .exception_table
        .iter()
        .map(|handler| {
            let catch = if handler.catch_type == CATCH_ALL_EXCEPTION_INDEX {
                CatchType::Any
            } else {
                CatchType::Type(
                    class
                        .constant_pool
                        .class_name(handler.catch_type)?
                        .to_owned(),
                )
            };
            Ok(SharedExceptionHandler {
                protected: AddressRange::new(
                    CodeAddress::from(handler.start_pc),
                    CodeAddress::from(handler.end_pc),
                ),
                handler: CodeAddress::from(handler.handler_pc),
                catch,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(FunctionBody::new(
        AddressUnit::Byte,
        instructions,
        exception_handlers,
    ))
}

#[cfg(test)]
mod tests {
    use disassembler::{BinaryFormat, DisassemblySource};

    use super::lower_class;
    use crate::bytecode::Opcode;
    use crate::classfile::{
        Attribute, ClassAccessFlags, ClassFile, CodeAttribute, Constant, ConstantPool,
        MethodAccessFlags, MethodInfo,
    };

    const JAVA_8_CLASS_MAJOR: u16 = 52;

    fn class_with_branch() -> ClassFile {
        let mut pool = ConstantPool::new();
        let class_name = pool.push_utf8("Example").unwrap();
        let this_class = pool
            .push(Constant::Class {
                name_index: class_name,
            })
            .unwrap();
        let super_name = pool.push_utf8("java/lang/Object").unwrap();
        let super_class = pool
            .push(Constant::Class {
                name_index: super_name,
            })
            .unwrap();
        let method_name = pool.push_utf8("choose").unwrap();
        let descriptor = pool.push_utf8("()V").unwrap();
        let code_name = pool.push_utf8("Code").unwrap();
        let code = vec![
            Opcode::IConst0.byte(),
            Opcode::IfEq.byte(),
            0,
            4,
            Opcode::Return.byte(),
            Opcode::Return.byte(),
        ];

        ClassFile {
            minor_version: 0,
            major_version: JAVA_8_CLASS_MAJOR,
            constant_pool: pool,
            access_flags: ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
            this_class,
            super_class,
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: vec![MethodInfo {
                access_flags: MethodAccessFlags::PUBLIC | MethodAccessFlags::STATIC,
                name_index: method_name,
                descriptor_index: descriptor,
                attributes: vec![Attribute::Code(CodeAttribute {
                    name_index: code_name,
                    max_stack: 1,
                    max_locals: 0,
                    code,
                    exception_table: Vec::new(),
                    attributes: Vec::new(),
                })],
            }],
            attributes: Vec::new(),
        }
    }

    #[test]
    fn lowers_jvm_methods_through_the_shared_source_trait() {
        let class = class_with_branch();
        let direct = lower_class(&class).unwrap();
        let through_trait = DisassemblySource::disassemble(&class).unwrap();

        assert_eq!(direct, through_trait);
        assert_eq!(direct.format, BinaryFormat::JavaClass);
        assert_eq!(direct.functions[0].symbol.owner, "Example");
        let graph = direct.functions[0]
            .body
            .as_ref()
            .unwrap()
            .control_flow_graph()
            .unwrap();
        assert_eq!(graph.cfg().num_blocks(), 3);
        assert_eq!(graph.cfg().num_edges(), 2);
    }
}
