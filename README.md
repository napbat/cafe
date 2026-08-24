# Cafe

Cafe is a Java-specific Rust workspace for lossless JVM class-file tooling and
shared bytecode analysis across Java ecosystem formats. It separates
format-specific parsing from shared disassembly and from the editable program
model, so consumers can use only the layer they need.

The workspace contains three library crates:

- `disassembler` defines instructions and references shared across Java
  bytecode formats, executable bodies, and cfglib-backed control-flow graphs.
- `cafe` provides Cafe's owned, dnlib-style program model: modules,
  types, fields, methods, editing, indexed lookup, and cross-module resolution.
- `java` parses and assembles JVM class files, decodes and encodes JVM
  bytecode, works with JARs, renders javap-like text, and adapts Java classes
  into the other two crates.

All three packages are libraries. Tool-specific command-line behavior belongs
in consuming repositories.

```text
JVM .class / JAR
       │
       ▼
      java ───────────► disassembler ───────────► cfglib CFGs
       │                     │
       └─────────────────────┴──────────────► cafe

future Java formats ─► format adapters ───────► shared layers
```

## Capabilities

`java` can discover JARs deterministically, inventory archive entries,
parse individual classes, and validate an entire archive. Its full validation
path parses and reassembles every class, decodes and re-encodes every method
body, checks descriptors and constant references, and constructs every shared
control-flow graph. Both binary round trips must reproduce the original bytes.

Unknown class-file attributes remain exact raw payloads. Modified UTF-8
constants retain their original UTF-16 units, including unpaired surrogates,
so unchanged classes remain byte-for-byte reproducible.

Cafe definitions retain format-qualified and overload-qualified identities.
Java adapters can load complete executable bodies or declarations only, which
keeps metadata-oriented consumers from paying for bytecode decoding.

## Java and JAR inspection

```rust
use java::jar::{JarFile, Traversal, discover_jars};

fn inspect(installation: &str) -> Result<(), java::Error> {
    for path in discover_jars(installation, Traversal::Recursive)? {
        let mut jar = JarFile::open(&path)?;
        println!("{}: {} classes", path.display(), jar.class_entry_count());

        for class in jar.class_summaries()? {
            println!("{} ({})", class.internal_name, class.major_version);
        }
    }

    Ok(())
}
```

Class-file assembly and bytecode encoding operate on public structured models:

```rust
use disassembler::DisassemblySource;
use java::{bytecode, disassemble, jar::JarFile};

fn inspect_class(jar_path: &str) -> Result<(), java::Error> {
    let mut jar = JarFile::open(jar_path)?;
    let class = jar.read_class("com.example.Application")?;

    for method in &class.methods {
        if let Some(code) = method.code() {
            let instructions = bytecode::decode_code(code)?;
            assert_eq!(bytecode::encode(&instructions)?, code.code);
        }
    }

    let shared = class.disassemble()?;
    for function in &shared.functions {
        if let Some(body) = &function.body {
            let graph = body.control_flow_graph()?;
            println!("{}: {} blocks", function.symbol.name, graph.cfg().num_blocks());
        }
    }

    let text = disassemble::disassemble(&class, &disassemble::Options::default())?;
    println!("assembled {} bytes\n{text}", class.to_bytes()?.len());
    Ok(())
}
```

## Cafe object model

Java classes can be lowered into independently owned modules and combined into
a program for traversal and resolution:

```rust
use cafe::{ModuleSource, Program};
use java::jar::JarFile;

fn load_program(jar_path: &str) -> Result<Program, java::Error> {
    let mut jar = JarFile::open(jar_path)?;
    let class = jar.read_class("com.example.Application")?;
    let module = class.to_module()?;
    Ok(Program::from_modules([module]))
}
```

For metadata-only tools, use
`java::cafe::lower_class_with_options` with
`MethodBodyMode::DeclarationsOnly` to skip method-body decoding.

## Workspace layout

```text
crates/
├── disassembler/       shared disassembly IR and CFG construction
│   ├── src/model/
│   ├── src/graph/
│   └── tests/
├── cafe/                owned definitions, identities, lookup, and resolution
│   ├── src/
│   └── tests/
└── java/                JVM class files, bytecode, JARs, and adapters
    ├── src/bytecode/
    ├── src/classfile/
    ├── src/disassembly/
    ├── src/cafe/
    ├── src/jar/
    └── tests/
```

Every source file is limited to 1,000 physical lines by a repository-wide
test. JVM-specified closed sets and policies use enums or typed bit flags;
fixed signatures, limits, masks, widths, and sentinels use named constants.

## Roadmap

See [future.md](future.md) for the JVM hardening plan, DEX frontend boundary,
optional Android runtime and Java Card work, and explicit non-goals.

## Build and verify

The repository follows the current stable Rust toolchain. With
[mise](https://mise.jdx.dev/) installed, `mise install` prepares the toolchain
and hook runner, and `mise run ci` runs the complete local gate. The equivalent
Cargo commands are:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

## License

Cafe is licensed under the [MIT License](LICENSE).
