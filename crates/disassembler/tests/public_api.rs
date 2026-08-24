//! Integration coverage for the public disassembly and graph API.

use disassembler::cfglib::{DominatorTree, verify, verify_edge_view};
use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CodeAddress, CodeSize, ControlFlowEdgeRole,
    Diagnostic, DiagnosticLevel, DiagnosticLocation, Diagnostics, FunctionBody, FunctionCoordinate,
    FunctionSymbol, Instruction, InstructionFlow, SourceMap,
};

const INSTRUCTION_SIZE: CodeSize = CodeSize::new(1);

fn instruction(address: u32, flow: InstructionFlow) -> Instruction {
    Instruction::new(
        CodeAddress::from(address),
        INSTRUCTION_SIZE,
        0,
        "fixture",
        Vec::new(),
        flow,
    )
}

fn coordinate(format: BinaryFormat, unit: AddressUnit) -> FunctionCoordinate {
    FunctionCoordinate::new(
        format,
        FunctionSymbol {
            owner: "sample/Example".to_owned(),
            name: "value".to_owned(),
            signature: "()I".to_owned(),
        },
        unit,
    )
}

#[test]
fn source_maps_support_expansion_and_reverse_lookup() {
    let source = coordinate(BinaryFormat::Dex, AddressUnit::CodeUnit16);
    let generated = coordinate(BinaryFormat::JavaClass, AddressUnit::Byte);
    let mut map = SourceMap::new(source, generated);
    let source_range = AddressRange::new(CodeAddress::new(2), CodeAddress::new(3));
    let generated_range = AddressRange::new(CodeAddress::new(5), CodeAddress::new(9));
    assert!(map.insert(source_range, generated_range).unwrap());
    assert!(!map.insert(source_range, generated_range).unwrap());
    assert_eq!(map.mappings_from(CodeAddress::new(2)).count(), 1);
    assert_eq!(map.mappings_to(CodeAddress::new(8)).count(), 1);
    assert!(map.mappings_to(CodeAddress::new(9)).next().is_none());

    let empty = AddressRange::new(CodeAddress::new(3), CodeAddress::new(3));
    assert!(map.insert(empty, generated_range).is_err());
}

#[test]
fn diagnostics_retain_typed_locations_and_severity() {
    let location = DiagnosticLocation::new(
        coordinate(BinaryFormat::Dex, AddressUnit::CodeUnit16),
        AddressRange::new(CodeAddress::new(4), CodeAddress::new(5)),
    );
    let diagnostic = Diagnostic::new(DiagnosticLevel::Error, "unsupported fixture")
        .with_code("fixture.unsupported")
        .at(location)
        .with_note("replace the fixture operation");
    let mut diagnostics = Diagnostics::new();
    diagnostics.push(diagnostic);
    assert!(diagnostics.has_errors());
    assert!(
        diagnostics.as_slice()[0]
            .to_string()
            .contains("fixture.unsupported")
    );
}

#[test]
fn downstream_consumers_can_use_cfglib_algorithms_on_shared_ir() {
    let body = FunctionBody::new(
        AddressUnit::Byte,
        vec![
            instruction(
                0,
                InstructionFlow::ConditionalBranch {
                    target: CodeAddress::from(2_u32),
                },
            ),
            instruction(1, InstructionFlow::Return),
            instruction(2, InstructionFlow::Return),
        ],
        Vec::new(),
    );

    let graph = body.control_flow_graph().unwrap();
    assert!(verify(graph.cfg()).is_ok());
    assert!(verify_edge_view(&graph.normal_view()).is_ok());

    let target = graph
        .block_for_instruction(CodeAddress::from(2_u32))
        .unwrap();
    let dominators = DominatorTree::compute(graph.cfg());
    assert!(dominators.dominates(graph.cfg().entry(), target));
    assert!(
        graph
            .cfg()
            .edges()
            .any(|edge| edge.payload().role() == &ControlFlowEdgeRole::ConditionalTaken)
    );
    assert!(graph.to_dot().contains("green4"));
}
