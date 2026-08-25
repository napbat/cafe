//! Cross-ISA target legalizations that require instruction expansion.

use cafe::{dex, disassembler, java, mlil};

const DEX_OWNER: &str = "Lsample/CrossIsa;";

fn multi_array_function() -> Result<mlil::Function, mlil::Error> {
    let source = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::JavaClass,
        disassembler::FunctionSymbol {
            owner: "sample/CrossIsa".to_owned(),
            name: "matrix".to_owned(),
            signature: "(II)[[I".to_owned(),
        },
        disassembler::AddressUnit::Byte,
    );
    let mut builder = mlil::FunctionBuilder::new(source);
    let rows = builder.declare_variable(mlil::VariableRole::Parameter(0), None)?;
    let columns = builder.declare_variable(mlil::VariableRole::Parameter(1), None)?;
    let result = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let body = builder.new_block("body");
    builder.add_edge(
        builder.entry(),
        body,
        mlil::EdgeMetadata::ordinary(mlil::EdgeRole::Entry),
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Allocate(mlil::AllocationKind::Array {
            array_type: mlil::ArrayType::new("[[I"),
            dimensions: 2,
        }),
        vec![
            mlil::TypedVariable::new(rows, mlil::ValueType::Integer),
            mlil::TypedVariable::new(columns, mlil::ValueType::Integer),
        ],
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("[[I".to_owned())),
        )],
        true,
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Return,
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("[[I".to_owned())),
        )],
        vec![],
        false,
        None,
    )?;
    builder.finish()
}

fn primitive_class_function() -> Result<mlil::Function, mlil::Error> {
    let source = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::Dex,
        disassembler::FunctionSymbol {
            owner: "Lsample/CrossIsa;".to_owned(),
            name: "integerClass".to_owned(),
            signature: "()Ljava/lang/Class;".to_owned(),
        },
        disassembler::AddressUnit::CodeUnit16,
    );
    let mut builder = mlil::FunctionBuilder::new(source);
    let result = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let body = builder.new_block("body");
    builder.add_edge(
        builder.entry(),
        body,
        mlil::EdgeMetadata::ordinary(mlil::EdgeRole::Entry),
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Constant(mlil::Constant::Reference(
            disassembler::Reference::resolved(disassembler::ReferenceKind::Type, 0, "I")
                .with_symbol(disassembler::ReferenceSymbol::Type("I".to_owned())),
        )),
        vec![],
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("Ljava/lang/Class;".to_owned())),
        )],
        true,
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Return,
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("Ljava/lang/Class;".to_owned())),
        )],
        vec![],
        false,
        None,
    )?;
    builder.finish()
}

fn intrinsic_identity_function() -> Result<mlil::Function, mlil::Error> {
    let source = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::JavaClass,
        disassembler::FunctionSymbol {
            owner: "sample/CrossIsa".to_owned(),
            name: "identity".to_owned(),
            signature: "(I)I".to_owned(),
        },
        disassembler::AddressUnit::Byte,
    );
    let mut builder = mlil::FunctionBuilder::new(source);
    let input = builder.declare_variable(mlil::VariableRole::Parameter(0), None)?;
    let result = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let body = builder.new_block("body");
    builder.add_edge(
        builder.entry(),
        body,
        mlil::EdgeMetadata::ordinary(mlil::EdgeRole::Entry),
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Intrinsic("identity".to_owned()),
        vec![mlil::TypedVariable::new(input, mlil::ValueType::Integer)],
        vec![mlil::TypedVariable::new(result, mlil::ValueType::Integer)],
        false,
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Return,
        vec![mlil::TypedVariable::new(result, mlil::ValueType::Integer)],
        vec![],
        false,
        None,
    )?;
    builder.finish()
}

fn intrinsic_void_function(may_throw: bool) -> Result<mlil::Function, mlil::Error> {
    let source = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::JavaClass,
        disassembler::FunctionSymbol {
            owner: "sample/CrossIsa".to_owned(),
            name: "intrinsic".to_owned(),
            signature: "()V".to_owned(),
        },
        disassembler::AddressUnit::Byte,
    );
    let mut builder = mlil::FunctionBuilder::new(source);
    let body = builder.new_block("body");
    builder.add_edge(
        builder.entry(),
        body,
        mlil::EdgeMetadata::ordinary(mlil::EdgeRole::Entry),
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Intrinsic("policy-test".to_owned()),
        vec![],
        vec![],
        may_throw,
        None,
    )?;
    builder.append_instruction(body, mlil::Operation::Return, vec![], vec![], false, None)?;
    builder.finish()
}

fn wide_initialized_array_function() -> Result<mlil::Function, mlil::Error> {
    let source = disassembler::FunctionCoordinate::new(
        disassembler::BinaryFormat::JavaClass,
        disassembler::FunctionSymbol {
            owner: "sample/CrossIsa".to_owned(),
            name: "longs".to_owned(),
            signature: "()[J".to_owned(),
        },
        disassembler::AddressUnit::Byte,
    );
    let mut builder = mlil::FunctionBuilder::new(source);
    let first = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let second = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let result = builder.declare_variable(mlil::VariableRole::Temporary, None)?;
    let body = builder.new_block("body");
    builder.add_edge(
        builder.entry(),
        body,
        mlil::EdgeMetadata::ordinary(mlil::EdgeRole::Entry),
        None,
    )?;
    for (variable, value) in [(first, 1), (second, 2)] {
        builder.append_instruction(
            body,
            mlil::Operation::Constant(mlil::Constant::Long(value)),
            vec![],
            vec![mlil::TypedVariable::new(variable, mlil::ValueType::Long)],
            false,
            None,
        )?;
    }
    builder.append_instruction(
        body,
        mlil::Operation::Allocate(mlil::AllocationKind::InitializedArray {
            array_type: mlil::ArrayType::new("[J"),
        }),
        vec![
            mlil::TypedVariable::new(first, mlil::ValueType::Long),
            mlil::TypedVariable::new(second, mlil::ValueType::Long),
        ],
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("[J".to_owned())),
        )],
        true,
        None,
    )?;
    builder.append_instruction(
        body,
        mlil::Operation::Return,
        vec![mlil::TypedVariable::new(
            result,
            mlil::ValueType::Reference(Some("[J".to_owned())),
        )],
        vec![],
        false,
        None,
    )?;
    builder.finish()
}

struct JavaIdentityIntrinsic;

impl java::mlil::JavaMlilIntrinsicLowerer for JavaIdentityIntrinsic {
    fn lower(
        &mut self,
        request: java::mlil::JavaIntrinsicRequest<'_>,
        _pool: &mut java::classfile::ConstantPool,
    ) -> Result<Vec<java::mlil::JavaIntrinsicInstruction>, java::mlil::JavaIntrinsicLoweringError>
    {
        assert_eq!(request.name, "identity");
        Ok(vec![java::mlil::JavaIntrinsicInstruction::new(
            java::bytecode::Opcode::Nop,
            java::bytecode::Operand::None,
        )])
    }
}

struct DexIdentityIntrinsic;

impl dex::mlil::DexMlilIntrinsicLowerer for DexIdentityIntrinsic {
    fn lower(
        &mut self,
        request: dex::mlil::DexIntrinsicRequest<'_>,
        _file: &dex::DexFile,
    ) -> Result<Vec<dex::mlil::DexIntrinsicInstruction>, dex::mlil::DexIntrinsicLoweringError> {
        assert_eq!(request.name, "identity");
        Ok(vec![dex::mlil::DexIntrinsicInstruction::new(
            dex::instruction::Opcode::Move16,
            dex::instruction::Operands::Registers {
                first: request.definition_registers[0],
                second: request.use_registers[0],
            },
        )])
    }
}

#[derive(Clone, Copy)]
enum InvalidIntrinsicExpansion {
    Empty,
    ControlFlow,
}

impl java::mlil::JavaMlilIntrinsicLowerer for InvalidIntrinsicExpansion {
    fn lower(
        &mut self,
        _request: java::mlil::JavaIntrinsicRequest<'_>,
        _pool: &mut java::classfile::ConstantPool,
    ) -> Result<Vec<java::mlil::JavaIntrinsicInstruction>, java::mlil::JavaIntrinsicLoweringError>
    {
        Ok(match self {
            Self::Empty => vec![],
            Self::ControlFlow => vec![java::mlil::JavaIntrinsicInstruction::new(
                java::bytecode::Opcode::Return,
                java::bytecode::Operand::None,
            )],
        })
    }
}

impl dex::mlil::DexMlilIntrinsicLowerer for InvalidIntrinsicExpansion {
    fn lower(
        &mut self,
        _request: dex::mlil::DexIntrinsicRequest<'_>,
        _file: &dex::DexFile,
    ) -> Result<Vec<dex::mlil::DexIntrinsicInstruction>, dex::mlil::DexIntrinsicLoweringError> {
        Ok(match self {
            Self::Empty => vec![],
            Self::ControlFlow => vec![dex::mlil::DexIntrinsicInstruction::new(
                dex::instruction::Opcode::ReturnVoid,
                dex::instruction::Operands::None,
            )],
        })
    }
}

#[test]
fn jvm_multianewarray_legalizes_to_verified_dalvik_loops() -> Result<(), Box<dyn std::error::Error>>
{
    const OWNER: &str = "Lsample/CrossIsa;";
    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let _ = builder.intern_type("[[I")?;
    let _ = builder.intern_type("[I")?;
    let _ = builder.intern_method_named(
        "Ljava/lang/NegativeArraySizeException;",
        "<init>",
        "V",
        &[],
    )?;
    let method = builder.intern_method_named(OWNER, "matrix", "[[I", &["I", "I"])?;
    let built = builder.build()?;
    let declaration = dex::file::EncodedMethod {
        method: built.indices.method(method).expect("method was interned"),
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };

    let lowered = dex::mlil::lower_body(&built.file, &multi_array_function()?)?;
    lowered.body.verify()?;
    let opcodes = lowered
        .body
        .instructions
        .iter()
        .filter_map(|instruction| instruction.encoding.data.opcode())
        .collect::<Vec<_>>();
    assert!(
        opcodes
            .iter()
            .filter(|&&opcode| opcode == dex::instruction::Opcode::NewArray)
            .count()
            >= 2
    );
    assert!(opcodes.contains(&dex::instruction::Opcode::AputObject));
    assert!(opcodes.contains(&dex::instruction::Opcode::IfGez));
    assert!(opcodes.contains(&dex::instruction::Opcode::Throw));

    let relifted = dex::mlil::lift_body(&built.file, &declaration, &lowered.body)?;
    assert!(relifted.verify().is_ok());
    Ok(())
}

#[test]
fn dalvik_primitive_class_literal_legalizes_to_jvm_wrapper_type_field()
-> Result<(), Box<dyn std::error::Error>> {
    let mut pool = java::classfile::ConstantPool::new();
    let lowered = java::mlil::lower_body(&primitive_class_function()?, &mut pool)?;
    lowered.body.verify()?;
    assert!(
        lowered
            .body
            .instructions
            .iter()
            .any(|instruction| instruction.encoding.opcode == java::bytecode::Opcode::GetStatic)
    );
    assert!(pool.iter().any(|(index, constant)| {
        matches!(constant, java::classfile::Constant::FieldRef { .. })
            && pool.describe(index).is_ok_and(|description| {
                description.contains("java/lang/Integer.TYPE:Ljava/lang/Class;")
            })
    }));
    let relifted = java::mlil::lift_body(
        &pool,
        "sample/CrossIsa",
        "integerClass",
        "()Ljava/lang/Class;",
        java::classfile::MethodAccessFlags::PUBLIC | java::classfile::MethodAccessFlags::STATIC,
        &lowered.body,
    )?;
    assert!(relifted.verify().is_ok());
    Ok(())
}

#[test]
fn explicit_intrinsic_policies_lower_to_both_targets() -> Result<(), Box<dyn std::error::Error>> {
    let function = intrinsic_identity_function()?;
    let mut pool = java::classfile::ConstantPool::new();
    let jvm = java::mlil::lower_body_with_resolver_and_intrinsics(
        &function,
        &mut pool,
        &mut java::DisplayJavaReferenceResolver,
        &mut JavaIdentityIntrinsic,
    )?;
    jvm.body.verify()?;

    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let method = builder.intern_method_named(DEX_OWNER, "identity", "I", &["I"])?;
    let built = builder.build()?;
    let declaration = dex::file::EncodedMethod {
        method: built.indices.method(method).expect("method was interned"),
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };
    let dalvik = dex::mlil::lower_body_with_resolver_and_intrinsics(
        &built.file,
        &function,
        &mut dex::mlil::TargetDexReferenceResolver,
        &mut DexIdentityIntrinsic,
    )?;
    dalvik.body.verify()?;
    let relifted = dex::mlil::lift_body(&built.file, &declaration, &dalvik.body)?;
    assert!(relifted.verify().is_ok());
    Ok(())
}

#[test]
fn intrinsic_policies_cannot_erase_throw_sites_or_change_control_flow()
-> Result<(), Box<dyn std::error::Error>> {
    let mut pool = java::classfile::ConstantPool::new();
    let error = java::mlil::lower_body_with_resolver_and_intrinsics(
        &intrinsic_void_function(true)?,
        &mut pool,
        &mut java::DisplayJavaReferenceResolver,
        &mut InvalidIntrinsicExpansion::Empty,
    )
    .unwrap_err();
    assert!(error.to_string().contains("empty expansion"), "{error}");
    let error = java::mlil::lower_body_with_resolver_and_intrinsics(
        &intrinsic_void_function(false)?,
        &mut pool,
        &mut java::DisplayJavaReferenceResolver,
        &mut InvalidIntrinsicExpansion::ControlFlow,
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-straight-line"), "{error}");

    let built = dex::file::DexBuilder::new(dex::DexVersion::V040).build()?;
    let error = dex::mlil::lower_body_with_resolver_and_intrinsics(
        &built.file,
        &intrinsic_void_function(true)?,
        &mut dex::mlil::TargetDexReferenceResolver,
        &mut InvalidIntrinsicExpansion::Empty,
    )
    .unwrap_err();
    assert!(error.to_string().contains("empty expansion"), "{error}");
    let error = dex::mlil::lower_body_with_resolver_and_intrinsics(
        &built.file,
        &intrinsic_void_function(false)?,
        &mut dex::mlil::TargetDexReferenceResolver,
        &mut InvalidIntrinsicExpansion::ControlFlow,
    )
    .unwrap_err();
    assert!(error.to_string().contains("non-straight-line"), "{error}");
    Ok(())
}

#[test]
fn wide_initialized_arrays_expand_to_dalvik_allocation_and_stores()
-> Result<(), Box<dyn std::error::Error>> {
    const OWNER: &str = "Lsample/CrossIsa;";
    let mut builder = dex::file::DexBuilder::new(dex::DexVersion::V040);
    let method = builder.intern_method_named(OWNER, "longs", "[J", &[])?;
    let built = builder.build()?;
    let declaration = dex::file::EncodedMethod {
        method: built.indices.method(method).expect("method was interned"),
        access_flags: dex::file::AccessFlags::STATIC,
        code: None,
    };
    let lowered = dex::mlil::lower_body(&built.file, &wide_initialized_array_function()?)?;
    lowered.body.verify()?;
    let opcodes = lowered
        .body
        .instructions
        .iter()
        .filter_map(|instruction| instruction.encoding.data.opcode())
        .collect::<Vec<_>>();
    assert!(opcodes.contains(&dex::instruction::Opcode::NewArray));
    assert!(opcodes.contains(&dex::instruction::Opcode::AputWide));
    assert!(!opcodes.contains(&dex::instruction::Opcode::FilledNewArrayRange));
    let relifted = dex::mlil::lift_body(&built.file, &declaration, &lowered.body)?;
    assert!(relifted.verify().is_ok());
    Ok(())
}
