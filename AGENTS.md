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
  control-flow graphs, plus format-qualified source maps and structured
  diagnostics. It must not depend on Cafe, Program, Java, or a native binary
  format.
- Keep `crates/program` neutral across Java ecosystem formats. It owns the
  editable `Program`/`Module`/definition model, identities, indexed lookup, and
  resolution. It may depend on the disassembler, but never on Cafe or a native
  frontend.
- Keep `crates/cafe` the only consumer entry point. It depends on and publicly
  re-exports every workspace capability under a concept-named namespace, while
  re-exporting Program's core types at its root. Every new feature crate must
  be wired into Cafe and its entry-point coverage in the same change. Focused
  implementation crates must not depend back on Cafe.
- Keep `crates/java` a library-only JVM frontend. It owns `.class` parsing and
  assembly, bytecode decoding and encoding, JAR utilities, read-only JMOD and
  JIMAGE ingestion, deterministic corpus validation, and adapters into the
  disassembler and Program. It also owns symbolic bytecode layout and JVM
  frame/stack-map analysis. It must not depend on Cafe. Do not add `src/main.rs`,
  Clap, or tool-specific output policy to this crate.
- Use the JAR entry reader for archive-wide operations. Validation, rewriting,
  and bulk consumers must share one ZIP reader rather than reopening the
  central directory or decompressing the same payload in separate passes.
- Keep `crates/dex` a library-only Android frontend. It owns DEX parsing and
  assembly, DEX 041 containers, CompactDex, Dalvik instruction decoding and
  encoding, APK/AAB provenance, corpus validation, executable semantics and
  register analysis, and adapters into the disassembler and Program. It must
  not depend on Cafe or leak Android-container details into shared lower layers.
- Use the APK or AAB entry reader for archive-wide operations. Bulk DEX
  consumers must select and visit artifacts through the single-reader APIs
  instead of reopening the ZIP directory per entry. Non-fail-fast validators
  should use raw-byte visitors so one malformed member does not stop traversal.
- Keep `crates/art` a library-only Android runtime boundary. It owns VDEX, ODEX,
  stable OAT metadata, quickening restoration, and adapters that canonicalize
  standard DEX before lowering. It may depend on DEX, disassembler, and Program,
  but never on Cafe. It must preserve CompactDex source identity and keep ELF,
  native instructions, architecture state, and runtime loading opaque.
- Keep `crates/jni` a safe, Java-specific linkage-metadata layer. It owns JNI
  descriptor-to-ABI mapping, exact symbol escaping, native declaration sets,
  safe explicit-registration resolution, portable header rendering, provenance
  reports, module native-access requirements, and adapters from Java, DEX, APK,
  and AAB artifacts. It must not depend on Cafe, load native libraries, expose
  raw pointer APIs, or absorb native instruction decoding.
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
- Keep Cafe's `src/lib.rs` a narrow documented facade. Cross-feature behavior
  belongs there only when it genuinely coordinates multiple focused crates.
- In Program, keep definitions, identities, modules, program storage,
  resolution, and source adapters in their own concept folders.
- In Java, keep `classfile/`, `bytecode/`, `jar/`, `disassembly/`, and `program/`
  independent. Keep frame analysis under `analysis/`, corpus reporting under
  `corpus/`, and JDK containers under `jmod/` and `jimage/`. Keep descriptors,
  textual presentation, crate errors, and public entry points at the source
  root.
- In DEX, keep `file/`, `instruction/`, `apk/`, `aab/`, `corpus/`,
  `disassembly/`, and `program/` independent. Keep CompactDex and DEX 041
  physical models under `file/`, and executable/register analysis under
  `analysis/`. Treat APK and AAB as container and provenance boundaries, not
  instruction sets.
- In ART, keep `vdex/`, `odex/`, `oat/`, and `quickening/` independent. Restore
  quickened standard DEX to canonical opcodes before using DEX adapters; retain
  CompactDex main/shared sections when complete canonicalization metadata is
  unavailable.
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
  method-handle kinds, primitive array types, DEX source encodings, ART
  container versions, quick opcodes, traversal modes, and body-loading modes.
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

## Shared disassembly and Program

- Retain native opcodes, signatures, addresses, table indices, access flags,
  references, and resolved display names needed by downstream consumers.
- Build and verify ordinary, branch, switch, legacy-subroutine, and exceptional
  control-flow edges through cfglib for every lowered executable body.
- Preserve normal-versus-exceptional edge meaning and native switch/catch
  provenance outside cfglib until its graph model can carry typed edge data.
- Keep source mappings format-qualified and capable of representing expansion
  and fusion. Diagnostics must retain overload-qualified native locations.
- Keep Program definition identity format-qualified and overload-qualified.
  Module mutation must preserve indexed lookup invariants; cross-module
  resolution must distinguish missing, unique, and ambiguous results.
- Adapters may offer explicit body-loading policies. Metadata-only consumers
  must not be forced to decode executable bodies.

## Verification

- Add focused tests for shared IR and graphs, Program ownership and resolution,
  native-format adapters, class parsing and assembly, bytecode, descriptors,
  symbolic layout, DEX semantics and registers, JVM frames and stack maps,
  source maps and diagnostics, JAR/JMOD/JIMAGE traversal, DEX 041 and
  CompactDex, APK/AAB traversal and rewrite reports, deterministic corpus
  validation, ART containers and quickening, JNI ABI mapping, symbol escaping,
  native overload and registration plans, header rendering, and Cafe-only
  access to every public workspace capability.
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
