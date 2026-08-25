use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, ExactText, FunctionCoordinate,
    FunctionSymbol, Reference, ReferenceKind, ReferenceSymbol,
};

use crate::{
    BinaryOperator, BranchOperandKind, BranchPredicate, CallKind, Constant, EdgeMetadata, EdgeRole,
    FunctionBuilder, Operation, Relation, TypedVariable, ValueType, VariableRole,
};

fn coordinate() -> FunctionCoordinate {
    FunctionCoordinate::new(
        BinaryFormat::JavaClass,
        FunctionSymbol {
            owner: "sample/Example".to_owned(),
            name: "value".to_owned(),
            signature: "(I)I".to_owned(),
        },
        AddressUnit::Byte,
    )
}

fn void_coordinate() -> FunctionCoordinate {
    FunctionCoordinate::new(
        BinaryFormat::JavaClass,
        FunctionSymbol {
            owner: "sample/Example".to_owned(),
            name: "run".to_owned(),
            signature: "()V".to_owned(),
        },
        AddressUnit::Byte,
    )
}

fn range(start: u64) -> AddressRange {
    AddressRange::new(CodeAddress::new(start), CodeAddress::new(start + 1))
}

#[test]
fn diamond_verifies_and_exposes_a_phi_in_derived_ssa() {
    let mut builder = FunctionBuilder::new(coordinate());
    let condition = builder
        .declare_variable(VariableRole::Parameter(0), None)
        .unwrap();
    let value = builder.declare_variable(VariableRole::Local, None).unwrap();
    let branch = builder.new_block("branch");
    let then_block = builder.new_block("then");
    let else_block = builder.new_block("else");
    let merge = builder.new_block("merge");

    builder
        .add_edge(
            builder.entry(),
            branch,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            branch,
            Operation::Branch(BranchPredicate {
                relation: Relation::NotEqual,
                operands: BranchOperandKind::IntegerZero,
            }),
            vec![TypedVariable::new(condition, ValueType::Integer)],
            vec![],
            false,
            Some(range(0)),
        )
        .unwrap();
    builder
        .add_edge(
            branch,
            then_block,
            EdgeMetadata::ordinary(EdgeRole::BranchTrue),
            Some(range(0)),
        )
        .unwrap();
    builder
        .add_edge(
            branch,
            else_block,
            EdgeMetadata::ordinary(EdgeRole::BranchFalse),
            Some(range(0)),
        )
        .unwrap();

    for (block, literal, address) in [(then_block, 1, 1), (else_block, 2, 2)] {
        builder
            .append_instruction(
                block,
                Operation::Constant(Constant::Integer(literal)),
                vec![],
                vec![TypedVariable::new(value, ValueType::Integer)],
                false,
                Some(range(address)),
            )
            .unwrap();
        builder
            .add_edge(
                block,
                merge,
                EdgeMetadata::ordinary(EdgeRole::FallThrough),
                Some(range(address)),
            )
            .unwrap();
    }
    builder
        .append_instruction(
            merge,
            Operation::Return,
            vec![TypedVariable::new(value, ValueType::Integer)],
            vec![],
            false,
            Some(range(3)),
        )
        .unwrap();

    let function = builder.finish().unwrap();
    assert!(function.verify().is_ok());
    assert_eq!(
        function
            .provenance()
            .mappings_from(CodeAddress::new(0))
            .count(),
        3
    );
    let ssa = function.ssa().unwrap();
    assert_eq!(ssa.block(merge).phis.len(), 1);
    assert_eq!(ssa.block(merge).phis[0].result.variable, value);
}

#[test]
#[allow(clippy::too_many_lines)]
fn semantic_analyses_run_on_the_canonical_payload_bearing_graph() {
    use disassembler::cfglib::{ConstValue, ProgramPoint};

    let mut builder = FunctionBuilder::new(coordinate());
    let left = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let right = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let sum = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let copied = builder.declare_variable(VariableRole::Local, None).unwrap();
    let dead = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let body = builder.new_block("body");
    let exit = builder.new_block("exit");
    builder
        .add_edge(
            builder.entry(),
            body,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    for (value, variable, address) in [(4, left, 0), (5, right, 1)] {
        builder
            .append_instruction(
                body,
                Operation::Constant(Constant::Integer(value)),
                vec![],
                vec![TypedVariable::new(variable, ValueType::Integer)],
                false,
                Some(range(address)),
            )
            .unwrap();
    }
    builder
        .append_instruction(
            body,
            Operation::Binary(BinaryOperator::Add),
            vec![
                TypedVariable::new(left, ValueType::Integer),
                TypedVariable::new(right, ValueType::Integer),
            ],
            vec![TypedVariable::new(sum, ValueType::Integer)],
            false,
            Some(range(2)),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Copy,
            vec![TypedVariable::new(sum, ValueType::Integer)],
            vec![TypedVariable::new(copied, ValueType::Integer)],
            false,
            Some(range(3)),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Constant(Constant::Integer(99)),
            vec![],
            vec![TypedVariable::new(dead, ValueType::Integer)],
            false,
            Some(range(4)),
        )
        .unwrap();
    builder
        .add_edge(
            body,
            exit,
            EdgeMetadata::ordinary(EdgeRole::FallThrough),
            Some(range(5)),
        )
        .unwrap();
    builder
        .append_instruction(
            exit,
            Operation::Return,
            vec![TypedVariable::new(copied, ValueType::Integer)],
            vec![],
            false,
            Some(range(5)),
        )
        .unwrap();

    let function = builder.finish().unwrap();
    let copy_site = ProgramPoint {
        block: body,
        inst_idx: 3,
    };
    let return_site = ProgramPoint {
        block: exit,
        inst_idx: 0,
    };
    let chains = function.def_use();
    assert!(chains.uses_of(copy_site).contains(&return_site));

    let liveness = function.liveness();
    assert!(liveness.is_live_out(&copied, body));
    assert!(liveness.is_live_in(&copied, exit));
    assert!(
        liveness
            .all_live_variables(function.cfg())
            .contains(&copied)
    );

    let constants = function.constants();
    assert_eq!(
        constants.fact_out(body).get(&copied),
        Some(&ConstValue::Const(Constant::Integer(9)))
    );
    assert_eq!(
        constants.fact_out(body).get(&dead),
        Some(&ConstValue::Const(Constant::Integer(99)))
    );
    let sparse = function.sparse_constants().unwrap();
    assert!(
        sparse
            .values
            .values()
            .any(|value| value == &ConstValue::Const(Constant::Integer(9)))
    );

    let expressions = function.expressions();
    let body_expressions = expressions
        .iter()
        .find(|expressions| expressions.block == body)
        .unwrap();
    let copied_expression = body_expressions
        .roots
        .iter()
        .find(|(variable, _)| *variable == copied)
        .unwrap();
    assert!(copied_expression.1.depth() >= 3);

    let dead_code = function.dead_code();
    assert_eq!(
        dead_code.instructions,
        vec![ProgramPoint {
            block: body,
            inst_idx: 4,
        }]
    );
    assert!(dead_code.unreachable_blocks.is_empty());
    let (without_dead_code, removed) = function.dead_code_eliminated_cfg();
    assert_eq!(removed, 1);
    assert_eq!(without_dead_code.block(body).instructions().len(), 4);

    let (rewritten, statistics) = function.copy_propagated_cfg();
    assert_eq!(statistics.uses_rewritten, 1);
    assert_eq!(statistics.copies_removed, 1);
    assert_eq!(rewritten.block(exit).instructions()[0].uses(), &[sum]);
    assert_eq!(function.cfg().block(body).instructions().len(), 5);
}

#[test]
fn verifier_rejects_an_invalid_operation_signature() {
    let mut builder = FunctionBuilder::new(coordinate());
    let value = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let body = builder.new_block("body");
    builder
        .add_edge(
            builder.entry(),
            body,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Nop,
            vec![TypedVariable::new(value, ValueType::Integer)],
            vec![],
            false,
            Some(range(0)),
        )
        .unwrap();

    let error = builder.finish().unwrap_err();
    assert!(error.to_string().contains("invalid typed signature"));
}

#[test]
fn verifier_checks_call_descriptors_against_typed_arguments() {
    let mut builder = FunctionBuilder::new(coordinate());
    let parameter = builder
        .declare_variable(VariableRole::Parameter(0), None)
        .unwrap();
    let body = builder.new_block("body");
    builder
        .add_edge(
            builder.entry(),
            body,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Call {
                kind: CallKind::Static,
                target: Reference::unresolved(ReferenceKind::Method, 3),
                descriptor: Some("(J)V".to_owned()),
            },
            vec![TypedVariable::new(parameter, ValueType::Integer)],
            vec![],
            false,
            Some(range(0)),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Return,
            vec![TypedVariable::new(parameter, ValueType::Integer)],
            vec![],
            false,
            Some(range(1)),
        )
        .unwrap();

    let error = builder.finish().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid typed signature for call")
    );
}

#[test]
fn verifier_distinguishes_polymorphic_target_and_effective_descriptors() {
    let source = FunctionCoordinate::new(
        BinaryFormat::Dex,
        FunctionSymbol {
            owner: "Lsample/Example;".to_owned(),
            name: "invoke".to_owned(),
            signature: "(Ljava/lang/invoke/MethodHandle;I)I".to_owned(),
        },
        AddressUnit::CodeUnit16,
    );
    let mut builder = FunctionBuilder::new(source);
    let receiver = builder
        .declare_variable(VariableRole::Parameter(0), None)
        .unwrap();
    let argument = builder
        .declare_variable(VariableRole::Parameter(1), None)
        .unwrap();
    let result = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let body = builder.new_block("body");
    builder
        .add_edge(
            builder.entry(),
            body,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    let target = Reference::resolved(
        ReferenceKind::Method,
        3,
        "java/lang/invoke/MethodHandle.invokeExact([Ljava/lang/Object;)Ljava/lang/Object;",
    )
    .with_symbol(ReferenceSymbol::Method {
        owner: "java/lang/invoke/MethodHandle".to_owned(),
        name: ExactText::new("invokeExact"),
        descriptor: "([Ljava/lang/Object;)Ljava/lang/Object;".to_owned(),
    });
    builder
        .append_instruction(
            body,
            Operation::Call {
                kind: CallKind::Polymorphic,
                target,
                descriptor: Some("(I)I".to_owned()),
            },
            vec![
                TypedVariable::new(
                    receiver,
                    ValueType::Reference(Some("Ljava/lang/invoke/MethodHandle;".to_owned())),
                ),
                TypedVariable::new(argument, ValueType::Integer),
            ],
            vec![TypedVariable::new(result, ValueType::Integer)],
            true,
            Some(range(0)),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Return,
            vec![TypedVariable::new(result, ValueType::Integer)],
            vec![],
            false,
            Some(range(1)),
        )
        .unwrap();

    builder.finish().unwrap();
}

#[test]
fn exception_edges_retain_exact_throw_sites_and_pre_state() {
    let mut builder = FunctionBuilder::new(void_coordinate());
    let caught = builder
        .declare_variable(VariableRole::Exception, None)
        .unwrap();
    let throwing = builder.new_block("invoke");
    let normal = builder.new_block("normal-return");
    let landing = builder.new_block("handler");
    builder
        .add_edge(
            builder.entry(),
            throwing,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    let throw_site = builder
        .append_instruction(
            throwing,
            Operation::Call {
                kind: CallKind::Static,
                target: Reference::unresolved(ReferenceKind::Method, 7),
                descriptor: Some("()V".to_owned()),
            },
            vec![],
            vec![],
            true,
            Some(range(4)),
        )
        .unwrap();
    builder
        .add_edge(
            throwing,
            normal,
            EdgeMetadata::ordinary(EdgeRole::FallThrough),
            Some(range(4)),
        )
        .unwrap();
    builder
        .add_edge(
            throwing,
            landing,
            EdgeMetadata::exceptional(
                EdgeRole::Exception {
                    catch: CatchType::Any,
                    handler_order: 0,
                    protected: AddressRange::new(CodeAddress::new(4), CodeAddress::new(5)),
                },
                throw_site,
            ),
            Some(range(4)),
        )
        .unwrap();
    builder
        .append_instruction(
            normal,
            Operation::Return,
            vec![],
            vec![],
            false,
            Some(range(5)),
        )
        .unwrap();
    builder
        .append_instruction(
            landing,
            Operation::CaughtException(CatchType::Any),
            vec![],
            vec![TypedVariable::new(
                caught,
                ValueType::Reference(Some("Ljava/lang/Throwable;".to_owned())),
            )],
            false,
            Some(range(6)),
        )
        .unwrap();
    builder
        .append_instruction(
            landing,
            Operation::Return,
            vec![],
            vec![],
            false,
            Some(range(6)),
        )
        .unwrap();

    let function = builder.finish().unwrap();
    let exception = function
        .cfg()
        .edges()
        .find(|edge| edge.payload().role.is_exception())
        .unwrap();
    assert_eq!(exception.payload().throw_site, Some(throw_site));
    assert!(
        function
            .ssa()
            .unwrap()
            .instruction(function.instruction_point(throw_site).unwrap())
            .is_some()
    );
}

#[test]
fn verifier_rejects_noncanonical_exact_reference_names() {
    let mut builder = FunctionBuilder::new(void_coordinate());
    let value = builder
        .declare_variable(VariableRole::Temporary, None)
        .unwrap();
    let block = builder.new_block("body");
    builder
        .add_edge(
            builder.entry(),
            block,
            EdgeMetadata::ordinary(EdgeRole::Entry),
            None,
        )
        .unwrap();
    let invalid = ValueType::Reference(Some("java/lang/Object".to_owned()));
    builder
        .append_instruction(
            block,
            Operation::Copy,
            vec![TypedVariable::new(value, invalid.clone())],
            vec![TypedVariable::new(value, invalid)],
            false,
            Some(range(0)),
        )
        .unwrap();
    builder
        .append_instruction(
            block,
            Operation::Return,
            vec![],
            vec![],
            false,
            Some(range(1)),
        )
        .unwrap();

    let error = builder.finish().unwrap_err();
    assert!(error.to_string().contains("invalid value type"));
}
