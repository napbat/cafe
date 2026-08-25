# Cafe

Cafe is the single consumer entry point for Java-specific binary tooling. One
dependency exposes lossless JVM class files and JDK containers, Android DEX and
application containers, ART runtime artifacts, shared disassembly and
control-flow graphs, an editable program model, and JNI linkage metadata
through coherent namespaces.

The workspace contains eight library crates. `cafe` is the public umbrella; the
other seven are focused implementation boundaries:

- `cafe` re-exports every supported capability and the complete program model.
- `program` owns modules, types, fields, methods, editing, indexed lookup, and
  cross-module resolution, plus the shared native-emitter contract.
- `classpath` normalizes JVM and DEX class identities into one declaration
  hierarchy and supplies native analysis views for both frontends.
- `disassembler` owns shared instructions, references, executable bodies,
  cfglib-backed control-flow graphs, format-qualified source maps, and
  structured diagnostics.
- `java` owns JVM class-file parsing and assembly, symbolic JVM bytecode
  construction, frame and stack-map analysis, JAR/JMOD/JIMAGE utilities,
  corpus validation, javap-like presentation, lowering into Program, and
  verified canonical emission back to class files.
- `dex` owns standard and CompactDex parsing and assembly, DEX 041 containers,
  Dalvik instructions, executable-body and register analysis, APK/AAB handling,
  corpus validation, provenance, lowering into Program, and verified canonical
  emission back to DEX.
- `art` owns VDEX/ODEX containers, stable OAT metadata, quickening restoration,
  and canonical DEX adapters without interpreting native code.
- `jni` owns native declarations, JNI ABI types, canonical symbols, explicit
  registration resolution, C headers, provenance reports, module native-access
  requirements, and Java/DEX extraction.

Tool-specific command-line behavior belongs in consuming repositories.
Consumers depend on `cafe`, not its implementation crates:

```toml
[dependencies]
cafe = { git = "https://github.com/napbat/cafe" }
```

```text
consumer
└── cafe
    ├── java             JVM .class, bytecode, JAR, JMOD, and JIMAGE
    ├── dex              DEX, CompactDex, APK, and AAB
    ├── art              VDEX, ODEX, OAT metadata, and dequickening
    ├── jni              native linkage metadata
    ├── classpath        unified JVM/DEX declaration hierarchy
    ├── disassembler     shared instruction IR and CFGs
    │   └── cfglib       graph algorithms
    └── program          owned definitions and resolution
```

## Capabilities

`cafe::java` can discover JARs deterministically, inventory archive entries,
parse individual classes, and validate an entire archive. Its full validation
path parses and reassembles every class, decodes and re-encodes every method
body, checks descriptors and constant references, and constructs every shared
control-flow graph. Archive metadata, resources, and classes are validated in
one payload pass using one ZIP reader. Both binary round trips must reproduce
the original bytes.

The same frontend reads JMOD archives and JIMAGE runtime images without
introducing another bytecode model. JMOD class traversal shares one archive
reader; JIMAGE supports both endiannesses, indexed resources, OpenJDK compact
constant-pool compression, and zlib-compressed payloads. The non-fail-fast
`java::corpus` runner accepts class, JAR, JMOD, and JIMAGE artifacts and reports
every failure with its artifact, entry, class, method, byte offset, and stage.

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

New JVM bodies can be built with labels instead of precomputed offsets.
`CodeBuilder` selects compact local and constant forms, aligns switches, widens
distant branches, expands distant conditional branches, and resolves symbolic
exception regions. `CodeAttribute::from_built_analyzed` then computes exact
stack/local maxima and emits deterministic full-frame stack maps. A
caller-supplied `ClassHierarchy` enables strict superclass and interface checks
when generated code references types outside the class being assembled.

Cafe definitions retain format-qualified and overload-qualified identities.
Java adapters can load complete executable bodies or declarations only, which
keeps metadata-oriented consumers from paying for bytecode decoding.

Program modules also have verified native backends. `java::emit_module`
rebuilds class files, re-interns structured constants instead of trusting stale
pool indices, recomputes verification frames and stack maps, and validates each
result. `dex::emit_module` deterministically rebuilds identifier tables,
instructions, exception tables, class data, and exact retained register-frame
resources before assembling and reparsing the result. Both frontends expose
stateful emitters for caller-selected version and reference-resolution policy.

`cafe::dex` retains native identifier tables, indices, code-unit addresses,
encoded values, annotations, debug programs, exception handlers, hidden-API
data, and map provenance. It parses DEX versions 035, 037, 038, 039, 040, and
041; assembles editable standalone versions 035 through 040; and parses,
transactionally edits, and assembles multi-header DEX 041 with `DexContainer`.
`DexBuilder` provides stable symbolic interning for nontrivial identifier
tables. Every standard Dalvik opcode and payload has matching decoding and
encoding, and validation covers descriptor, index, register-width, invocation,
branch, payload, and exception constraints.

DEX executable analysis is available independently of any output format.
Every standard opcode has typed register uses, definitions, result behavior,
and throw behavior. Body analysis associates switch and array payloads,
move-result producers, and handler entries; operation-level control flow keeps
payloads non-executable and adds exceptional edges only for operations that can
throw. Fixed-point register analysis tracks parameters, wide pairs, constants,
arrays, fields, invocations, constructor initialization, exception pre-states,
and caller-supplied classpath hierarchy relationships. Indexed instruction and
encoded-value resolvers retain owned symbols and exact Java UTF-16 names.

`cafe::classpath::ClasspathHierarchy` builds one open type world from class
files, DEX files, Program modules, JAR/JMOD/JIMAGE images, or APK/AAB DEX sets.
JVM internal names and DEX object descriptors normalize to one canonical
identity; equivalent declarations merge and incompatible duplicates fail
transactionally. Borrowed `jvm_view()` and `dex_view()` adapters implement the
native hierarchy contracts used by JVM frame and DEX register analysis.

Shared `SourceMap` and `Diagnostics` models provide format-qualified provenance
and machine-readable reporting for consumers that generate or transform
bytecode. These are infrastructure APIs only: Cafe does not currently include a
DEX-to-JVM instruction translator or a DEX-to-JAR workflow.

APK support is a lossless archive boundary around DEX artifacts rather than a
separate instruction set. It provides stable entry identities, deterministic
multidex ordering, exact pristine output, typed signature-block IDs, and
explicit reject, preserve, or strip policies for signature material during
rewrites. `RewriteReport` identifies both blocking signature conditions and ZIP
metadata that configured encoders cannot reproduce exactly.

CompactDex 001 support retains an explicit source-format identity and split
main/shared-data representation. Typed headers, feature flags, offset tables,
method locations, debug offsets, and compact code items have matching checked
decode/encode APIs. Android App Bundles provide deterministic module-qualified
DEX discovery and one-reader traversal. The non-fail-fast `dex::corpus` runner
validates standalone DEX, DEX 041 containers, APKs, and AABs while retaining
artifact, entry, method, code-unit offset, and stage diagnostics.

`cafe::art` handles runtime-produced Android artifacts without leaking ART
state into canonical DEX. It validates VDEX 009, 012, 020, 021, and sectioned
027, restores quickened standard opcodes before shared lowering, preserves
CompactDex split sections explicitly, and parses/preserves ODEX 036. OAT
support discovers the stable metadata prefix in direct or ELF-contained input;
architecture-specific fields and native instructions remain opaque.

`cafe::jni` preserves exact Java UTF-16 names while parsing method descriptors
into typed Java and JNI ABI values. It implements the specification's short
and long symbol forms, escape-failure rules, and short-then-long lookup order.
Binding plans use long symbols only when native declarations with the same
owner and name are overloaded; non-native overloads do not affect the plan.
The crate also resolves caller-supplied `RegisterNatives` tables through opaque
implementation keys, renders policy-controlled portable C headers, retains
class/JAR/DEX/APK/AAB origins in aggregate reports, selects effective
multi-release JAR classes for a target release, and reports Java 24-and-later
module native-access requirements. It remains a safe metadata boundary and
does not load libraries, expose raw pointers, or analyze native machine code.

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

Shared executable graphs use caller-payload-aware cfglib CFGs. Every edge keeps
its stable identity, exact source and target address, and detailed role:
conditional arm, switch default or signed case key, legacy subroutine call or
call-site continuation, or ordered exception handler with its protected range
and catch type. Protected instructions are isolated before exception edges are
added, so edge-sensitive analyses can use instruction pre-state for exceptional
flow and post-state for ordinary flow. `ControlFlowGraph::normal_view()` filters
exception edges without rebuilding or renumbering the graph; the full CFG and
normal-only view can be passed directly to cfglib algorithms.

The native JVM and DEX analysis graphs implement cfglib's borrowed node, edge,
and rooted view contracts without replacing their bytecode-offset APIs. JVM
verification frames and Dalvik register frames now run through cfglib's seeded,
fallible edge-sensitive solver; format-specific transfer and merge errors retain
their original locations, ordinary edges propagate post-state, and exception
edges propagate the required pre-state.

Shared control-flow graphs expose complementary raw and recovered exception
models. `ControlFlowGraph::exception_model()` derives cfglib landing pads,
protected sources, and stable exception-edge identities. Each distinct native
protected range is registered as a cfglib region with exact protected blocks
and ordered handler entry/kind. The graph-owned `HandlerTypes` registry maps
every stable `HandlerRef` back to its exact resolved catch descriptor. Because
JVM and DEX tables do not encode complete handler-body extents, the canonical
region records
`HandlerBody::Unknown` rather than silently treating a guessed end as native
metadata.

`ControlFlowGraph::recovered_exception_model()` adds a conservative analysis
layer without mutating that canonical graph. It retains each exact native
handler definition in table order, maps its `ExceptionHandlerIndex` directly
to a cfglib `HandlerRef`, and reports the stable `EdgeId`, source block, and
native address of every represented exceptional transfer. Handler ownership
contains only blocks reachable from that entry and unreachable from the method
entry or another distinct handler entry. Shared continuation blocks remain
explicit boundaries, while normal entry paths, cross-handler paths, external
predecessors, indirect transfers, and missing branch arms are reported as
ambiguities. Catch-all handlers are classified by observable bytecode exits;
`ThrowingCleanup` means every represented exit from an isolated recovered body
throws, not that the original exception is proven to be rethrown or that the
source language used `finally`.

`ControlFlowGraph::recovered_structured_control_flow()` is the explicit bridge
to cfglib lifting. It clones the canonical graph and promotes a recovered
handler body only when its extent is complete and nonambiguous and does not
overlap another promoted body. Shared or ambiguous handlers remain
unstructured, the canonical `HandlerBody::Unknown` metadata never changes, and
catch-all handlers are never relabeled as source-level `finally`.

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
            println!("{}: {} blocks", function.symbol.name, graph.cfg().block_count());
        }
    }

    let text = disassemble::disassemble(&class, &disassemble::Options::default())?;
    println!("assembled {} bytes\n{text}", class.to_bytes()?.len());
    Ok(())
}
```

## Bytecode construction and analysis

Symbolic construction keeps branch offsets and stack-map details out of the
consumer's policy code:

```rust
use cafe::java;
use cafe::java::{analysis::ClassHierarchy, bytecode, classfile};

fn generated_method() -> Result<classfile::CodeAttribute, java::Error> {
    let mut pool = classfile::ConstantPool::new();
    let mut builder = bytecode::CodeBuilder::new();
    let _ = builder.emit_load(bytecode::LocalKind::Integer, 0);
    let _ = builder.emit(bytecode::Opcode::IConst1, bytecode::Operand::None);
    let _ = builder.emit(bytecode::Opcode::IAdd, bytecode::Operand::None);
    let _ = builder.emit(bytecode::Opcode::IReturn, bytecode::Operand::None);
    let built = builder.finish()?;

    let hierarchy = ClassHierarchy::new();
    let (code, analysis) = classfile::CodeAttribute::from_built_analyzed_with_hierarchy(
        &mut pool,
        "sample/Generated",
        "increment",
        "(I)I",
        classfile::MethodAccessFlags::STATIC,
        &built,
        &hierarchy,
    )?;
    assert_eq!((analysis.max_stack(), analysis.max_locals()), (2, 1));
    Ok(code)
}
```

For DEX, `cafe::dex::analysis` exposes structural body analysis, typed control
flow, exact symbol resolution, and method register analysis. The default method
entry point derives a hierarchy from the enclosing file;
`analyze_method_registers_with_hierarchy` accepts classpath relationships for
external types.

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

The same owned module can feed a unified hierarchy and a native backend:

```rust
use cafe::{ModuleSource, Program, classpath::ClasspathHierarchy, java};

fn rebuild(class: &java::classfile::ClassFile) -> Result<(), Box<dyn std::error::Error>> {
    let module = class.to_module()?;
    let program = Program::from_modules([module.clone()]);
    let hierarchy = ClasspathHierarchy::from_program(&program)?;
    let emitted = java::emit_module(&module)?;

    assert_eq!(hierarchy.len(), 1);
    assert_eq!(emitted.len(), 1);
    Ok(())
}
```

Use `dex::emit_module` for a DEX-qualified module. References are resolved from
their structured symbols rather than their source indices. Recursive bootstrap,
method-handle, or call-site structures not retained by Program are rejected
explicitly instead of being guessed.

## DEX and APK inspection

APK members retain their exact archive origin when they are parsed, lowered to
shared disassembly, or converted into Cafe modules:

```rust
use cafe::dex;
use cafe::dex::{
    ProgramOptions,
    apk::{ApkFile, DexVisitControl},
};

fn inspect_apk(path: &str) -> Result<(), dex::Error> {
    let apk = ApkFile::open(path)?;

    apk.visit_dex(
        |_| true,
        |artifact| -> Result<DexVisitControl, dex::Error> {
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
            Ok(DexVisitControl::Continue)
        },
    )
}
```

`visit_dex` validates canonical contiguous multidex provenance, selects entries
before decompression, supports early stopping, and keeps one ZIP reader alive
for the complete visit. `read_all_dex` provides the collecting convenience API
over the same single-pass implementation. `visit_dex_bytes` is the raw-payload
variant used by corpus tools that must continue after a malformed member.
`cafe::dex::aab::AabFile` provides the corresponding module-qualified APIs for
Android App Bundles.

Structured DEX edits are assembled through `DexFile::to_bytes`. APK rewrites
require an explicit signature policy whenever existing v1 or signing-block
material could be invalidated; inspect `ApkFile::rewrite_report` before saving
when reproducible ZIP metadata matters. Use `DexContainer` for physical DEX 041
multi-header files and `CompactDexFile` when source CompactDex identity and
shared data must remain intact.

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
`native_methods_in_aab` covers module-qualified bundle members, and the
`binding_report` variants retain exact artifact provenance.
`NativeMethod::registration` exposes the exact name-and-descriptor key needed
by explicit registration even when the conventional symbol escape is
unavailable. `RegisterNativesTable::resolve` safely associates those keys with
opaque consumer implementation IDs, while `render_c_header` generates portable
declarations without introducing pointer or loader APIs.

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
│   ├── src/source_map.rs
│   ├── src/diagnostic.rs
│   └── tests/
├── classpath/           unified JVM/DEX declarations and hierarchy views
│   └── src/
├── java/                JVM class files, bytecode, JARs, and adapters
│   ├── src/analysis/
│   ├── src/bytecode/
│   ├── src/classfile/
│   ├── src/corpus/
│   ├── src/disassembly/
│   ├── src/jar/
│   ├── src/jimage/
│   ├── src/jmod/
│   ├── src/program/
│   └── tests/
├── dex/                 DEX/CompactDex, Android archives, and adapters
│   ├── src/analysis/
│   ├── src/aab/
│   ├── src/apk/
│   ├── src/corpus/
│   ├── src/disassembly/
│   ├── src/file/
│   ├── src/instruction/
│   ├── src/program/
│   └── tests/
├── art/                 VDEX, ODEX, OAT metadata, and quickening
│   ├── src/oat/
│   ├── src/odex/
│   ├── src/quickening/
│   └── src/vdex/
└── jni/                 JNI metadata, reports, headers, and adapters
    ├── src/binding/
    ├── src/descriptor/
    ├── src/dex/
    ├── src/header/
    ├── src/java/
    ├── src/method/
    ├── src/report/
    └── src/symbol/
```

Every source file is limited to 1,000 physical lines by a repository-wide
test. JVM-specified closed sets and policies use enums or typed bit flags;
fixed signatures, limits, masks, widths, and sentinels use named constants.

## Roadmap

See [future.md](future.md) for the completed hardening baseline, the explicitly
deferred DEX-to-JVM transformation boundary, conditional Java Card support, and
Java-specific non-goals.

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
