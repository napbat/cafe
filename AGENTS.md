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
- Put future Android support in a sibling `crates/dex` package. It should
  implement the existing disassembler and Cafe boundaries without mixing DEX
  implementation details into another crate.
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
- Use named constants for fixed signatures, attribute names, limits, widths,
  sentinels, masks, archive suffixes, and semantic string values. Do not leave
  unexplained numeric or string literals in production logic.
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
  and JAR traversal.
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
