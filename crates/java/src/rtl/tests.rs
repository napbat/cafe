//! JVM LLIL/RTL/MLIL integration coverage.

use mlil::{BinaryOperator, EdgeRole, Operation};

use super::{lift_body, lift_method, lower_body, raise_function};
use crate::bytecode::Opcode;
use crate::classfile::{
    Attribute, CATCH_ALL_EXCEPTION_INDEX, ClassAccessFlags, ClassFile, CodeAttribute, Constant,
    ConstantPool, ExceptionHandler, MethodAccessFlags, MethodInfo,
};
use crate::llil;

const JAVA_8_CLASS_MAJOR: u16 = 52;

fn arithmetic_class() -> ClassFile {
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
    let descriptor = pool.push_utf8("(I)I").unwrap();
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
                max_stack: 2,
                max_locals: 1,
                code: vec![
                    Opcode::ILoad0.byte(),
                    Opcode::IConst1.byte(),
                    Opcode::IAdd.byte(),
                    Opcode::IReturn.byte(),
                ],
                exception_table: Vec::new(),
                attributes: Vec::new(),
            })],
        }],
        attributes: Vec::new(),
    }
}

fn finally_class() -> ClassFile {
    let mut class = arithmetic_class();
    let Attribute::Code(code) = &mut class.methods[0].attributes[0] else {
        unreachable!()
    };
    code.max_stack = 2;
    code.max_locals = 3;
    code.code = vec![
        Opcode::IConst0.byte(),
        Opcode::IStore1.byte(),
        Opcode::ILoad0.byte(),
        Opcode::IConst2.byte(),
        Opcode::IMul.byte(),
        Opcode::IStore1.byte(),
        Opcode::Goto.byte(),
        0,
        10,
        Opcode::AStore2.byte(),
        Opcode::ILoad0.byte(),
        Opcode::IConst1.byte(),
        Opcode::IAdd.byte(),
        Opcode::IStore1.byte(),
        Opcode::ALoad2.byte(),
        Opcode::AThrow.byte(),
        Opcode::ILoad1.byte(),
        Opcode::IReturn.byte(),
    ];
    code.exception_table = vec![ExceptionHandler {
        start_pc: 2,
        end_pc: 6,
        handler_pc: 9,
        catch_type: CATCH_ALL_EXCEPTION_INDEX,
    }];
    class
}

#[test]
fn jvm_llil_rtl_mlil_and_back_preserve_semantics() {
    let mut class = arithmetic_class();
    let access_flags = class.methods[0].access_flags;
    let body = llil::Body::from_code(class.methods[0].code().unwrap()).unwrap();
    let rtl = lift_body(
        &class.constant_pool,
        "Example",
        "value",
        "(I)I",
        access_flags,
        &body,
    )
    .unwrap();
    assert_eq!(rtl.signature().parameters.len(), 1);
    assert_eq!(rtl.signature().returns.len(), 1);

    let semantic = raise_function(&rtl).unwrap();
    assert!(semantic.verify().is_ok());
    assert!(semantic.instructions().any(|instruction| {
        matches!(
            instruction.operation(),
            Operation::Binary(BinaryOperator::Add)
        )
    }));

    let lowered = lower_body(&rtl, &mut class.constant_pool).unwrap();
    lowered.body.verify().unwrap();
    let relifted = lift_body(
        &class.constant_pool,
        "Example",
        "value",
        "(I)I",
        access_flags,
        &lowered.body,
    )
    .unwrap();
    assert!(raise_function(&relifted).unwrap().verify().is_ok());
}

#[test]
fn jvm_rtl_preserves_exact_exception_edges_and_regions() {
    let mut class = arithmetic_class();
    let void_descriptor = class.constant_pool.push_utf8("()V").unwrap();
    class.methods[0].descriptor_index = void_descriptor;
    let Attribute::Code(code) = &mut class.methods[0].attributes[0] else {
        unreachable!()
    };
    code.max_stack = 1;
    code.max_locals = 0;
    code.code = vec![
        Opcode::AConstNull.byte(),
        Opcode::CheckCast.byte(),
        0,
        u8::try_from(class.this_class).unwrap(),
        Opcode::Pop.byte(),
        Opcode::Return.byte(),
    ];
    code.exception_table = vec![ExceptionHandler {
        start_pc: 1,
        end_pc: 4,
        handler_pc: 4,
        catch_type: class.this_class,
    }];

    let rtl = lift_method(&class, &class.methods[0]).unwrap().unwrap();
    let exceptional: Vec<_> = rtl
        .cfg()
        .edges()
        .filter(|edge| matches!(edge.payload().role, EdgeRole::Exception { .. }))
        .collect();
    assert_eq!(exceptional.len(), 1);
    assert!(exceptional[0].payload().throw_site.is_some());
    assert_eq!(rtl.cfg().regions().len(), 1);
    assert!(matches!(
        rtl.cfg().regions()[0].handlers[0].body,
        cfglib::HandlerBody::Unknown
    ));

    let semantic = raise_function(&rtl).unwrap();
    assert!(semantic.verify().is_ok());
    assert!(
        semantic
            .cfg()
            .edges()
            .any(|edge| edge.payload().role == EdgeRole::Commit)
    );
    assert!(
        semantic.instructions().any(|instruction| {
            matches!(instruction.operation(), Operation::CaughtException(_))
        })
    );
}

#[test]
fn unentered_finally_handler_keeps_caught_exception_type() {
    let class = finally_class();
    let rtl = lift_method(&class, &class.methods[0]).unwrap().unwrap();
    let semantic = raise_function(&rtl).expect("disconnected handler state remains typed");
    assert!(semantic.verify().is_ok());
}
