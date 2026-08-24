//! Integration coverage for the public Cafe ownership and resolution API.

use cafe::{MethodId, Module, Program, Resolution, TypeId};
use disassembler::{BinaryFormat, Disassembly, Function, FunctionSymbol, RawAccessFlags};

const SAMPLE_ACCESS_FLAG_BITS: u32 = 1;

fn declaration(module: &str) -> Disassembly {
    Disassembly {
        format: BinaryFormat::JavaClass,
        name: module.to_owned(),
        functions: vec![Function {
            symbol: FunctionSymbol {
                owner: "sample/Type".to_owned(),
                name: "run".to_owned(),
                signature: "()V".to_owned(),
            },
            access_flags: RawAccessFlags::new(SAMPLE_ACCESS_FLAG_BITS),
            body: None,
        }],
    }
}

#[test]
fn builds_navigable_owned_definitions_from_disassembly() {
    let module = Module::try_from(declaration("sample/Type")).unwrap();
    let owner = TypeId::new(BinaryFormat::JavaClass, "sample/Type");
    let method = MethodId::new("run", "()V");

    assert_eq!(module.type_count(), 1);
    assert_eq!(module.method_count(), 1);
    assert_eq!(
        module
            .type_definition(&owner)
            .unwrap()
            .method(&method)
            .unwrap()
            .access_flags()
            .bits(),
        SAMPLE_ACCESS_FLAG_BITS
    );
}

#[test]
fn reports_ambiguous_definitions_across_modules() {
    let program =
        Program::from_disassemblies([declaration("first.class"), declaration("second.class")])
            .unwrap();
    let owner = TypeId::new(BinaryFormat::JavaClass, "sample/Type");

    assert_eq!(
        program.resolve_type(&owner),
        Resolution::Ambiguous { matches: 2 }
    );
}
