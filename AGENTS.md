# Repository instructions

These rules apply to the entire repository.

## Crate boundaries

- Cafe is Java-specific. Shared layers may abstract across JVM class files,
  DEX, CompactDex, and a future Java Card frontend, but must not absorb CLR,
  WebAssembly, scripting-VM, or native-ISA concerns.
- Use a virtual Cargo workspace at the repository root. Put every package under
  `crates/<concept>` and follow Cargo's conventional project layout.
- Keep `crates/disassembler` neutral across Java ecosystem formats. It owns raw
  disassembly IR, the `DisassemblySource` adapter contract, and cfglib-backed
  control-flow graphs. It must not depend on Cafe, Java, or a native binary
  format.
- Keep `crates/cafe` neutral across Java ecosystem formats. It owns the
  editable dnlib-style `Program`/`Module`/definition model, identities, lookup,
  and resolution. It may depend on the disassembler, but never on Java or
  another native frontend.
- Keep `crates/java` a library-only JVM frontend. It owns `.class` parsing and
  assembly, bytecode decoding and encoding, JAR utilities, and adapters into
  the disassembler and Cafe. Do not add `src/main.rs`, Clap, or tool-specific
  output policy to this crate.
- Keep `crates/dex` a library-only Android frontend. It owns DEX parsing and
  assembly, Dalvik instruction decoding and encoding, APK and multidex
  provenance, and adapters into the disassembler and Cafe. Do not leak DEX or
  APK implementation details into another crate.
- Keep `crates/jni` a safe, Java-specific linkage-metadata layer. It owns JNI
  descriptor-to-ABI mapping, exact symbol escaping, native declaration sets,
  explicit registration keys, and adapters from Java and DEX artifacts. It
  must not load native libraries, expose raw pointer APIs, or absorb native
  instruction decoding.
- Keep each package name identical to its directory name and declare every
  shared dependency once under `[workspace.dependencies]`.

## Source organization

- Use `src/lib.rs` for library entry points, `src/main.rs` only for a package's
  single default executable, `src/bin/` for additional executables, `tests/`
  for integration tests, `examples/` for examples, and `benches/` for
  benchmarks.
- Split files by concept and keep every source file at or below 1,000 physical
  lines, including tests and documentation comments. Split before the limit;
  do not compress formatting or combine unrelated concepts to evade it.
- When a concept needs multiple implementation files, give it a directory
  with a narrow `mod.rs` facade and plainly named child modules. Do not place
  ad-hoc `<concept>_<concern>.rs` siblings beside `<concept>.rs`.
- In Java, keep `classfile/`, `bytecode/`, `jar/`, `disassembly/`, and `cafe/`
  independent. Keep descriptors, textual presentation, crate errors, and
  public entry points at the source root.
- In DEX, keep `file/`, `instruction/`, `apk/`, `disassembly/`, and `cafe/`
  independent. Treat APK as a container and provenance boundary, not another
  instruction set.
- In JNI, keep `descriptor/`, `method/`, `symbol/`, `binding/`, `java/`, and
  `dex/` independent. Preserve exact UTF-16 precursors through descriptor
  parsing and symbol escaping.
- Preserve established public API paths with narrow re-export facades when an
  implementation moves between modules.
- Document every public contract. Body comments explain constraints and design
  reasons, not syntax, change history, or decorative sections.

## Modeling rules

- Give format-specified and policy values descriptive types. Use enums for
  closed sets such as binary formats, JVM opcodes, constant-pool tags,
  method-handle kinds, primitive array types, traversal modes, and body-loading
  modes.
- Use typed bit-flag wrappers for combinable access flags.
- Use named constants or dedicated types for every format-defined signature,
  tag, opcode, debug event, field offset, flag, limit, width, alignment,
  sentinel, mask, archive suffix, and semantic string value. Raw numeric and
  string literals are only appropriate for ordinary calculations and test
  fixtures; on-disk protocol meaning must never live in a magic literal.
- Keep parsing and assembly, and bytecode decoding and encoding,
  feature-complete together. A newly supported parsed structure or opcode must
  have a matching encoding path.
- Preserve unknown class-file attributes and exact modified UTF-8/UTF-16 data
  so unchanged class files remain lossless through parse/assemble round trips.

## Shared disassembly and Cafe

- Retain native opcodes, signatures, addresses, table indices, access flags,
  references, and resolved display names needed by downstream consumers.
- Build and verify ordinary, branch, switch, legacy-subroutine, and exceptional
  control-flow edges through cfglib for every lowered executable body.
- Keep Cafe definition identity format-qualified and overload-qualified.
  Module mutation must preserve indexed lookup invariants; cross-module
  resolution must distinguish missing, unique, and ambiguous results.
- Adapters may offer explicit body-loading policies. Metadata-only consumers
  must not be forced to decode executable bodies.

## Verification

- Add focused tests for shared IR and graphs, Cafe ownership and resolution,
  native-format adapters, class parsing and assembly, bytecode, descriptors,
  JAR traversal, JNI ABI mapping, symbol escaping, and native overload plans.
- Require all of these commands to pass:

  ```text
  cargo fmt --all -- --check
  cargo check --workspace --all-targets --locked
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
  ```

- Enforce the 1,000-line source limit with a repository-wide check.
- Update `README.md`, `future.md`, and this file whenever scope, crate
  boundaries, package names, or public capabilities change.
