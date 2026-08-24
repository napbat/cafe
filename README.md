# Cafe

Cafe is the single consumer entry point for Java-specific binary tooling. One
dependency exposes lossless JVM class files and JARs, Android DEX and APKs,
shared disassembly and control-flow graphs, an editable program model, and JNI
linkage metadata through coherent namespaces.

The workspace contains six library crates. `cafe` is the public umbrella; the
other five are focused implementation boundaries:

- `cafe` re-exports every supported capability and the complete program model.
- `program` owns modules, types, fields, methods, editing, indexed lookup, and
  cross-module resolution.
- `disassembler` owns shared instructions, references, executable bodies, and
  cfglib-backed control-flow graphs.
- `java` owns JVM class-file parsing and assembly, JVM bytecode, JAR utilities,
  javap-like presentation, and lowering into shared layers.
- `dex` owns DEX parsing and assembly, Dalvik instructions, APK editing,
  multidex provenance, and lowering into shared layers.
- `jni` owns native declarations, JNI ABI types, canonical symbols, explicit
  registration keys, and Java/DEX extraction.

Tool-specific command-line behavior belongs in consuming repositories.
Consumers depend on `cafe`, not its implementation crates:

```toml
[dependencies]
cafe = { git = "https://github.com/napbat/cafe" }
```

```text
consumer
└── cafe
    ├── java             JVM .class, bytecode, and JAR
    ├── dex              Android DEX, Dalvik, and APK
    ├── jni              native linkage metadata
    ├── disassembler     shared instruction IR and CFGs
    │   └── cfglib       graph algorithms
    └── program          owned definitions and resolution
```

## Capabilities

`cafe::java` can discover JARs deterministically, inventory archive entries,
parse individual classes, and validate an entire archive. Its full validation
path parses and reassembles every class, decodes and re-encodes every method
body, checks descriptors and constant references, and constructs every shared
control-flow graph. Both binary round trips must reproduce the original bytes.

Unknown class-file attributes remain exact raw payloads. Modified UTF-8
constants retain their original UTF-16 units, including unpaired surrogates,
so unchanged classes remain byte-for-byte reproducible.

Every standard JVM attribute through Java 26 has a typed, editable model,
including stack maps, bootstrap methods, annotations, modules, nests, records,
and permitted subclasses. Constant-pool interning and class/member constructors
avoid manual index bookkeeping. Method-body edits are transactional: callers
either retain an unchanged instruction layout, explicitly discard code
metadata, or provide a `BytecodeOffsetMap` that remaps exception handlers,
stack maps, debugging ranges, and code-level type annotations together.

Cafe definitions retain format-qualified and overload-qualified identities.
Java adapters can load complete executable bodies or declarations only, which
keeps metadata-oriented consumers from paying for bytecode decoding.

`cafe::dex` retains native identifier tables, indices, code-unit addresses,
encoded values, annotations, debug programs, exception handlers, hidden-API
data, and map provenance. It parses DEX versions 035, 037, 038, 039, 040, and
041, and assembles editable standalone versions 035 through 040. Every standard
Dalvik opcode and payload has matching decoding and encoding, and validation
covers descriptor, index, register-width, invocation, branch, payload, and
exception constraints.

APK support is a lossless archive boundary around DEX artifacts rather than a
separate instruction set. It provides stable entry identities, deterministic
multidex ordering, exact pristine output, typed signature-block IDs, and
explicit reject, preserve, or strip policies for signature material during
rewrites.

`cafe::jni` preserves exact Java UTF-16 names while parsing method descriptors
into typed Java and JNI ABI values. It implements the specification's short
and long symbol forms, escape-failure rules, and short-then-long lookup order.
Binding plans use long symbols only when native declarations with the same
owner and name are overloaded; non-native overloads do not affect the plan.
The crate is a safe metadata boundary and does not load libraries, expose raw
pointers, or analyze native machine code.

## Java and JAR inspection

```rust
use cafe::java;
use cafe::java::jar::{JarFile, Traversal, discover_jars};

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

For large archives, `visit_classes` keeps one ZIP reader alive across the
selected entries. Selection happens before decompression and class parsing,
and the visitor can stop without touching later entries:

```rust
use cafe::java;
use cafe::java::jar::{ClassVisitControl, JarFile};

fn inspect_until(path: &str, stop_after: &str) -> Result<(), java::Error> {
    let jar = JarFile::open(path)?;
    jar.visit_classes(
        |entry| {
            entry.name != "module-info.class"
                && !entry.name.ends_with("/package-info.class")
        },
        |entry, class| -> Result<ClassVisitControl, java::Error> {
            println!("{}: {} methods", entry.name, class.methods.len());
            Ok(if entry.name == stop_after {
                ClassVisitControl::Stop
            } else {
                ClassVisitControl::Continue
            })
        },
    )
}
```

The callback may use any consumer error type implementing `From<java::Error>`.
Archive and parser failures retain the exact physical entry name.

JARs are also fully editable. Entry IDs remain stable across renames and
reordering, unchanged archives serialize byte-for-byte, and rewrites retain
entry metadata, archive order, comments, manifests, service configurations,
and multi-release overlays:

```rust
use cafe::java;
use cafe::java::jar::{JarFile, Manifest, SignaturePolicy};

fn rewrite(input: &str, output: &str) -> Result<(), java::Error> {
    let mut jar = JarFile::open(input)?;
    jar.put_file("assets/config.json", br#"{"enabled":true}"#.to_vec())?;
    jar.rename_entry("old/name.txt", "new/name.txt")?;

    let mut manifest = jar.manifest()?.unwrap_or_else(Manifest::new);
    manifest.main_mut().set("Implementation-Title", "Cafe")?;
    jar.set_manifest(&manifest)?;
    jar.set_multi_release(true)?;
    jar.add_versioned_file(17, "assets/config.json", b"java 17".to_vec())?;

    jar.validate_archive()?;
    jar.save_with_signature_policy(output, SignaturePolicy::Strip)
}
```

The default save policy refuses to rewrite a signed JAR. Callers must choose
to preserve potentially stale signature entries or strip the signature files
and manifest digests explicitly.

Class-file assembly and bytecode encoding operate on public structured models:

```rust
use cafe::disassembler::DisassemblySource;
use cafe::java;
use cafe::java::{bytecode, disassemble, jar::JarFile};

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
use cafe::{ModuleSource, Program, java};
use cafe::java::jar::JarFile;

fn load_program(jar_path: &str) -> Result<Program, java::Error> {
    let mut jar = JarFile::open(jar_path)?;
    let class = jar.read_class("com.example.Application")?;
    let module = class.to_module()?;
    Ok(Program::from_modules([module]))
}
```

For metadata-only tools, use
`cafe::java::lower_class_with_options` with `cafe::java::ProgramOptions` and
`cafe::java::MethodBodyMode::DeclarationsOnly` to skip method-body decoding.

## DEX and APK inspection

APK members retain their exact archive origin when they are parsed, lowered to
shared disassembly, or converted into Cafe modules:

```rust
use cafe::dex;
use cafe::dex::{ProgramOptions, apk::ApkFile};

fn inspect_apk(path: &str) -> Result<(), dex::Error> {
    let apk = ApkFile::open(path)?;

    for artifact in apk.read_all_dex()? {
        println!(
            "{}: DEX {:?}",
            artifact.origin.entry_name,
            artifact.file.version()
        );
        let disassembly = artifact.disassemble()?;
        let module = artifact.to_module(ProgramOptions::default())?;
        println!(
            "{} functions, {} types",
            disassembly.functions.len(),
            module.type_count()
        );
    }

    Ok(())
}
```

Structured DEX edits are assembled through `DexFile::to_bytes`. APK rewrites
require an explicit signature policy whenever existing v1 or signing-block
material could be invalidated.

## JNI declaration inspection

JNI declarations can be extracted directly from the Java and DEX frontend
models. Each binding retains its typed Java declaration and selected export
symbol:

```rust
use cafe::java::jar::JarFile;
use cafe::jni;

fn inspect_natives(path: &str) -> Result<(), jni::Error> {
    let jar = JarFile::open(path)?;
    let class = jar.read_class("com.example.NativeCodec")?;
    let methods = jni::java::native_methods(&class)?;

    for binding in methods.bindings()? {
        let prototype = binding.method().prototype();
        println!(
            "{} -> {} returns {}",
            binding.method().id(),
            binding.symbol(),
            prototype.return_type().c_name()
        );
    }
    Ok(())
}
```

Use `cafe::jni::dex::native_methods` for one `DexFile` or
`cafe::jni::dex::native_methods_in_apk` for a complete canonical multidex set.
`NativeMethod::registration` exposes the exact name-and-descriptor key needed
by explicit registration even when the conventional symbol escape is
unavailable.

## Workspace layout

```text
crates/
├── cafe/                single public entry point
│   ├── src/lib.rs
│   └── tests/
├── program/             owned definitions, identities, lookup, and resolution
│   ├── src/definition/
│   ├── src/identity/
│   ├── src/module/
│   ├── src/program/
│   └── tests/
├── disassembler/        shared disassembly IR and CFG construction
│   ├── src/model/
│   ├── src/graph/
│   └── tests/
├── java/                JVM class files, bytecode, JARs, and adapters
│   ├── src/bytecode/
│   ├── src/classfile/
│   ├── src/disassembly/
│   ├── src/program/
│   ├── src/jar/
│   └── tests/
├── dex/                 DEX files, Dalvik bytecode, APKs, and adapters
│   ├── src/file/
│   ├── src/instruction/
│   ├── src/disassembly/
│   ├── src/program/
│   ├── src/apk/
│   └── tests/
└── jni/                 JNI declarations, ABI types, symbols, and adapters
    ├── src/descriptor/
    ├── src/method/
    ├── src/symbol/
    ├── src/binding/
    ├── src/java/
    ├── src/dex/
    └── tests/
```

Every source file is limited to 1,000 physical lines by a repository-wide
test. JVM-specified closed sets and policies use enums or typed bit flags;
fixed signatures, limits, masks, widths, and sentinels use named constants.

## Roadmap

See [future.md](future.md) for JVM and DEX hardening, optional Android runtime
containers and Java Card work, and explicit Java-specific non-goals.

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
