//! Focused JVM LLIL-to-MLIL adapter coverage.

use ::mlil::{CallKind, EdgeRole, Operation, SourceStorage, ValueType};

use super::lift_method;
use crate::bytecode::Opcode;
use crate::classfile::{
    Attribute, ClassAccessFlags, ClassFile, CodeAttribute, Constant, ConstantPool,
    ExceptionHandler, MethodAccessFlags, MethodInfo,
};

const JAVA_8_CLASS_MAJOR: u16 = 52;

fn class_with_method(
    descriptor_text: &str,
    code: Vec<u8>,
    max_stack: u16,
    max_locals: u16,
    exception_table: Vec<ExceptionHandler>,
) -> ClassFile {
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
    let method_name = pool.push_utf8("value").unwrap();
    let descriptor = pool.push_utf8(descriptor_text).unwrap();
    let code_name = pool.push_utf8("Code").unwrap();
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
                max_stack,
                max_locals,
                code,
                exception_table,
                attributes: Vec::new(),
            })],
        }],
        attributes: Vec::new(),
    }
}

#[test]
fn lifts_stack_arithmetic_into_generic_variables_and_ssa() {
    let class = class_with_method(
        "(I)I",
        vec![
            Opcode::ILoad0.byte(),
            Opcode::IConst1.byte(),
            Opcode::IAdd.byte(),
            Opcode::IReturn.byte(),
        ],
        2,
        1,
        vec![],
    );
    let function = lift_method(&class, &class.methods[0]).unwrap().unwrap();

    assert!(function.verify().is_ok());
    assert!(function.ssa().unwrap().phis().next().is_none());
    assert!(
        function
            .cfg()
            .blocks()
            .iter()
            .flat_map(disassembler::cfglib::BasicBlock::instructions)
            .any(|instruction| {
                matches!(
                    instruction.operation(),
                    Operation::Binary(::mlil::BinaryOperator::Add)
                )
            })
    );
    assert!(function.variables().iter().any(|variable| {
        variable
            .native
            .is_some_and(|native| matches!(native.storage, SourceStorage::JvmLocal(0)))
    }));
}

#[test]
fn canonicalizes_stack_array_operands() {
    let class = class_with_method(
        "([III)I",
        vec![
            Opcode::ALoad0.byte(),
            Opcode::ILoad1.byte(),
            Opcode::ILoad2.byte(),
            Opcode::IAStore.byte(),
            Opcode::ALoad0.byte(),
            Opcode::ILoad1.byte(),
            Opcode::IALoad.byte(),
            Opcode::IReturn.byte(),
        ],
        3,
        3,
        vec![],
    );
    let function = lift_method(&class, &class.methods[0]).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let store = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .find(|instruction| {
            matches!(
                instruction.operation(),
                Operation::Array {
                    access: ::mlil::ArrayAccess::Put,
                    ..
                }
            )
        })
        .unwrap();
    assert!(store.use_types()[0].is_reference());
    assert_eq!(store.use_types()[1], ValueType::Integer);
    assert_eq!(store.use_types()[2], ValueType::Integer);
    function.ssa().unwrap();
}

#[test]
fn protected_definitions_commit_only_on_normal_flow() {
    let class = class_with_method(
        "()V",
        vec![
            Opcode::AConstNull.byte(),
            Opcode::CheckCast.byte(),
            0,
            2,
            Opcode::Pop.byte(),
            Opcode::Return.byte(),
        ],
        1,
        0,
        vec![ExceptionHandler {
            start_pc: 1,
            end_pc: 4,
            handler_pc: 4,
            catch_type: 2,
        }],
    );
    let function = lift_method(&class, &class.methods[0]).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let roles: Vec<_> = function
        .cfg()
        .edges()
        .map(|edge| &edge.payload().role)
        .collect();
    assert_eq!(
        roles
            .iter()
            .filter(|role| matches!(role, EdgeRole::Exception { .. }))
            .count(),
        1
    );
    assert!(roles.contains(&&EdgeRole::Commit));
    assert!(roles.iter().any(|role| {
        matches!(
            role,
            EdgeRole::Exception {
                catch: disassembler::CatchType::Type(descriptor),
                ..
            } if descriptor == "LExample;"
        )
    }));
    assert!(
        function
            .cfg()
            .blocks()
            .iter()
            .flat_map(disassembler::cfglib::BasicBlock::instructions)
            .any(|instruction| matches!(instruction.operation(), Operation::CaughtException(_)))
    );
    function.ssa().unwrap();
}

#[test]
fn constructor_calls_refine_every_preserved_alias() {
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
    let method_name = pool.push_utf8("make").unwrap();
    let descriptor = pool.push_utf8("()LExample;").unwrap();
    let code_name = pool.push_utf8("Code").unwrap();
    let init_name = pool.push_utf8("<init>").unwrap();
    let init_descriptor = pool.push_utf8("()V").unwrap();
    let init_name_and_type = pool
        .push(Constant::NameAndType {
            name_index: init_name,
            descriptor_index: init_descriptor,
        })
        .unwrap();
    let constructor = pool
        .push(Constant::MethodRef {
            class_index: this_class,
            name_and_type_index: init_name_and_type,
        })
        .unwrap();
    let [class_high, class_low] = this_class.to_be_bytes();
    let [constructor_high, constructor_low] = constructor.to_be_bytes();
    let class = ClassFile {
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
                max_stack: 2,
                max_locals: 0,
                code: vec![
                    Opcode::New.byte(),
                    class_high,
                    class_low,
                    Opcode::Dup.byte(),
                    Opcode::InvokeSpecial.byte(),
                    constructor_high,
                    constructor_low,
                    Opcode::AReturn.byte(),
                ],
                exception_table: Vec::new(),
                attributes: Vec::new(),
            })],
        }],
        attributes: Vec::new(),
    };
    let function = lift_method(&class, &class.methods[0]).unwrap().unwrap();

    assert!(function.verify().is_ok());
    let instructions = function
        .cfg()
        .blocks()
        .iter()
        .flat_map(disassembler::cfglib::BasicBlock::instructions)
        .collect::<Vec<_>>();
    assert!(instructions.iter().any(|instruction| {
        matches!(
            instruction.operation(),
            Operation::Call {
                kind: CallKind::Special,
                ..
            }
        )
    }));
    let refinement = instructions
        .iter()
        .find(|instruction| matches!(instruction.operation(), Operation::TypeRefine))
        .unwrap();
    assert!(matches!(
        refinement.use_types(),
        [ValueType::Uninitialized { .. }]
    ));
    assert!(matches!(
        refinement.def_types(),
        [ValueType::Reference(Some(descriptor))] if descriptor == "LExample;"
    ));
    function.ssa().unwrap();
}
