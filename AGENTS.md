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
  resolution, plus the format-neutral `ModuleEmitter` contract. It may depend
  on the disassembler, but never on Cafe or a native frontend.
- Keep `crates/mlil` neutral across Java ecosystem formats. Cfglib owns the
  generic MLIL function, instruction, identity, provenance, checked-builder,
  verification scaffold, and reusable analysis integration; this crate owns
  Cafe's Java-managed dialect: typed semantic values and operations, exact edge
  payloads, format-qualified native coordinates, analysis hooks, semantic
  verification, and compatibility aliases over `cfglib::mlil`. It may depend
  on cfglib and the disassembler, but never on Cafe or a native frontend. It
  must not move JVM descriptors, Dalvik zero values, native table references,
  catch types, or source formats into cfglib, nor own native instruction
  decoding, LLIL encoding, whole-artifact DEX-to-JVM policy, or decompiler
  presentation.
- Keep `crates/classpath` as the cross-format Java type-world aggregator. It
  normalizes JVM internal names and DEX object descriptors, merges equivalent
  declarations, diagnoses conflicting declarations, and supplies explicit JVM
  and DEX hierarchy views. It may depend on Java, DEX, and Program, but never
  on Cafe or container-specific policy.
- Keep `crates/decompiler` as the focused JVM-class-to-Java-source presentation
  layer. It lifts executable methods through verified MLIL, owns Java source
  rendering policies, canonical pass ordering and opt-in presentation-pass
  profiles, generic-signature presentation, metadata-proven member
  class compilation units, caller-supplied hierarchy and method-exception
  environments, structured recovery diagnostics, and
  generated-source provenance, and may depend on Java, MLIL, and the
  disassembler. Local and anonymous class placement remains method-level
  recovery rather than a `$`-name guess. It must not depend on Cafe, mutate
  input class files, absorb DEX-to-JAR coordination, or guess unsupported
  semantics without an explicit diagnostic and conservative stub.
- Keep `crates/cafe` the only library consumer entry point. It depends on and
  publicly re-exports every workspace capability under a concept-named
  namespace, while re-exporting Program's core types at its root. Every new
  feature crate must be wired into Cafe and its entry-point coverage in the
  same change. Focused implementation crates must not depend back on Cafe.
- Keep `crates/cafe-cli` the thin first-party application boundary. It depends
  on the Cafe facade rather than focused implementation crates and owns Clap
  syntax, filesystem output policy, human-readable reporting, and process exit
  status. Java parsing, archive selection, hierarchy modeling, and source
  recovery semantics remain in their focused libraries.
- Keep `crates/java` a library-only JVM frontend. It owns `.class` parsing and
  assembly, bytecode decoding and encoding, JAR utilities, read-only JMOD and
  JIMAGE ingestion, deterministic corpus validation, and adapters into the
  disassembler and Program. It also owns symbolic bytecode layout,
  JVM-specific LLIL/RTL and checked native round trips, direct
  JVM-LLIL-to-RTL lifting, cfglib RTL-to-MLIL raising and MLIL-to-RTL lowering,
  verified semantic lowering to JVM LLIL, JVM frame/stack-map analysis, and
  verified canonical Program-to-class-file emission. Keep its
  native instruction graph adapted to cfglib's rooted edge view and run frame
  propagation through the fallible seeded edge solver. It must not depend on
  Cafe. Do not add `src/main.rs`, Clap, or tool-specific output policy to this
  crate.
- Use the JAR entry reader for archive-wide operations. Validation, rewriting,
  and bulk consumers must share one ZIP reader rather than reopening the
  central directory or decompressing the same payload in separate passes.
- Keep `crates/dex` a library-only Android frontend. It owns DEX parsing and
  assembly, DEX 041 containers, CompactDex, Dalvik instruction decoding and
  encoding, Dalvik-specific LLIL/RTL and checked native round trips, direct
  Dalvik-LLIL-to-RTL lifting, cfglib RTL-to-MLIL raising and MLIL-to-RTL lowering,
  verified semantic lowering to Dalvik LLIL, APK/AAB provenance, corpus validation,
  executable semantics and register analysis, and adapters into the
  disassembler and Program, including verified canonical Program-to-DEX
  emission. Keep its native operation graph adapted to cfglib's
  rooted edge view and run register propagation through the fallible seeded
  edge solver. It must not depend on Cafe or leak Android-container details
  into shared lower layers.
- Use the APK or AAB entry reader for archive-wide operations. Bulk DEX
  consumers must select and visit artifacts through the single-reader APIs
  instead of reopening the ZIP directory per entry. Non-fail-fast validators
  should use raw-byte visitors so one malformed member does not stop traversal.
- Keep `crates/art` a library-only Android runtime boundary. It owns VDEX, ODEX,
  stable OAT metadata, quickening restoration, and adapters that canonicalize
  standard DEX before lifting. It may depend on DEX, disassembler, and Program,
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
- Keep Cafe CLI's `src/main.rs` a narrow executable entry point. Split command
  syntax, archive orchestration, output policy, and fatal errors by concept.
- In Program, keep definitions, identities, modules, program storage,
  resolution, and source adapters in their own concept folders.
- In MLIL, keep the Java-managed dialect, descriptors and verification,
  shared RTL semantic-operation adaptation, analysis hooks, operations, types,
  effects, and edges independent. Reuse
  cfglib's generic storage rather than shadowing its function, instruction,
  identity, variable, provenance, error, or builder types. Keep the crate's
  root a narrow compatibility and re-export facade.
- In Classpath, keep canonical declaration models, hierarchy queries, native
  views, ingestion, and errors independent.
- In Decompiler, keep class declarations, method control recovery, instruction
  rendering, variable layout, Java naming, diagnostics, and generated source
  maps independent. Keep the crate root a narrow documented facade.
- In Java, keep `classfile/`, `bytecode/`, `llil/`, `rtl/`, `mlil/`, `jar/`,
  `disassembly/`, and `program/` independent. Keep frame analysis under
  `analysis/`, corpus
  reporting under `corpus/`, and JDK containers under `jmod/` and `jimage/`.
  Keep descriptors, textual presentation, crate errors, and public entry points
  at the source root.
- In DEX, keep `file/`, `instruction/`, `apk/`, `aab/`, `corpus/`,
  `disassembly/`, `llil/`, `rtl/`, `mlil/`, and `program/` independent. Keep CompactDex
  and DEX 041 physical models under `file/`, and executable/register analysis
  under `analysis/`. Treat APK and AAB as container and provenance boundaries,
  not instruction sets.
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
- Keep JVM LLIL in Java and Dalvik LLIL in DEX. Do not make either frontend's
  stack or register mechanics the shared semantic model, and do not translate
  directly between the two LLILs. Lift each into its frontend-owned RTL, then
  raise both RTLs into MLIL as their generic convergence layer.
- Name representation transformations by direction: `lift` means moving from
  native bytecode into shared disassembly, Program, LLIL, RTL, or from RTL into
  MLIL; `lower` means moving back toward target RTL, LLIL, and native encodings.
  Helpers, diagnostics, and documentation must use the same convention.
- Keep JVM and Dalvik RTL dialects distinct, including native storage and
  verifier constraints. Cfglib owns generic checked RTL/MLIL bridge storage and
  rewrite maps; the `mlil` crate may share only Java-managed operation mapping.
  Fallible edge translation must preserve exact switch, continuation, catch,
  protected-range, and throw-site identity. Translate signatures, exception
  regions, and many-to-many provenance in both directions, and never retain
  generated target storage or synthetic temporaries as source-native provenance.
- Separate every LLIL instruction's normalized semantic operation from its
  exact native encoding provenance. Native lifting must validate the complete
  stream, reverse conversion must reject stale semantic/encoding pairs and run
  the native codec, and complete-body adapters must retain and revalidate exact
  handler, debug, stack-map, register-resource, and source-offset metadata.
- Keep MLIL operand order canonical and independent of native evaluation or
  encoding order: receivers precede field values, while arrays and indices
  precede array values. Derive point-specific types from verified JVM frames or
  DEX register states rather than guessing from opcodes alone.
- Canonicalize exact MLIL object and array value types to JVM-compatible
  descriptors across all frontends. Preserve Dalvik's exact zero/null lattice
  value rather than forcing it prematurely into either integer or reference
  semantics. Model array allocation and initialization with exact semantic
  descriptors and typed values, not JVM allocation-form flags or raw DEX
  payload bytes. Keep declared call targets separate from effective call-site
  descriptors where signature-polymorphic dispatch requires both.
- Model definitions produced by protected throwing operations through explicit
  normal-flow commit blocks. Exceptional edges must retain the pre-instruction
  native state, ordered catch metadata, protected ranges, and exact MLIL throw
  sites. Materialize delivered exceptions at synthetic handler landings; do not
  infer handler-body extents or source-level `finally`.
- Keep RTL and MLIL provenance deterministic, many-to-many, format-qualified, and able
  to represent native instruction expansion and payload fusion. Verify every
  function before exposing dominance or SSA. Keep MLIL-to-RTL-to-LLIL lowering
  in the owning target frontend, treat source format only as provenance, reject
  invalid or target-unencodable semantics, rebuild target resources and ordered
  exception metadata, omit stale offset-based metadata, and return
  generated-native source mappings. Default lowerers must use symbolic target
  linkage; source-index reuse belongs in explicit same-source entry points.
  Treat lowering as canonical semantic generation, not byte-identical replay of
  LLIL encoding provenance.
- Preserve unknown class-file attributes and exact modified UTF-8/UTF-16 data
  so unchanged class files remain lossless through parse/assemble round trips.
- Recover Java source only from verified MLIL. Structure dominance-proven
  reducible flow, retain exact ordered exception dispatch through a Java-valid
  state machine when necessary, and map generated spans through stable MLIL
  identities to native provenance. Unsupported dynamic linkage,
  synchronization structure, or declaration sugar must produce deterministic
  diagnostics rather than invented source semantics.
- Run decompiler normalization through cfglib's named pass pipeline over one
  derived presentation graph. Keep pass selection explicit and dependency-safe,
  and never mutate the verified canonical MLIL function.
- On direct parsed-class paths, decode each JVM method once and share the
  checked native stream across LLIL classification, frame propagation, and RTL
  lifting. Keep public editable-LLIL entry points checked independently.
- Index generated-fragment line boundaries once when translating source maps;
  do not rescan an entire rendered body per mapped span. Schedule bounded JAR
  workers deterministically with the largest estimated bytecode units first so
  pathological methods overlap ordinary compilation units.

## Shared disassembly and Program

- Retain native opcodes, signatures, addresses, table indices, access flags,
  references, reconstructable structured symbols, resolved display names, and
  exact register-frame resources needed by downstream consumers.
- Build and verify ordinary, branch, switch, legacy-subroutine, and exceptional
  control-flow edges through cfglib for every lifted executable body.
- Expose cfglib's generic exception-flow model from shared graphs. Register
  exact protected block sets and ordered handler entry/kind as regions, mark
  handler extents explicitly unknown when the native format omits them, and
  keep exact native exception provenance in stable edge payloads.
- Offer a separate recovered exception model that maps exact native handler
  indices to cfglib handler identities, claims only blocks exclusive to one
  handler entry, and reports shared boundaries and ambiguity evidence. Never
  promote recovered extents to native `HandlerBody::Known` metadata or equate
  catch-all throwing behavior with a proven rethrow or source-level `finally`.
- Offer an explicit derived structured-lifting bridge that clones the CFG and
  promotes only complete, nonambiguous, nonoverlapping recovered handler bodies
  to `HandlerBody::Known`. Never mutate the canonical graph or synthesize
  source-level `finally` semantics.
- Carry normal-versus-exceptional meaning, native switch keys, ordered catch
  metadata, exact throw sites, and legacy continuation call sites in cfglib
  edge payloads. Offer zero-copy normal-only views over the same stable edge
  identities, and isolate protected instructions when exceptional pre-state
  precision requires instruction-granular blocks.
- Keep source mappings format-qualified and capable of representing expansion
  and fusion. Diagnostics must retain overload-qualified native locations.
- Keep Program definition identity format-qualified and overload-qualified.
  Module mutation must preserve indexed lookup invariants; cross-module
  resolution must distinguish missing, unique, and ambiguous results.
- Re-intern structured Program references when emitting JVM or DEX artifacts;
  source table indices are diagnostic provenance, not valid output indices.
  Emitted artifacts must pass their native assembly, parse, and analysis gates.
- Normalize JVM and DEX class names only in Classpath. Native analysis APIs
  consume explicit borrowed hierarchy views rather than absorbing the other
  frontend's naming or container model.
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
  access to every public workspace capability. Cover canonical Program-to-JVM
  and Program-to-DEX emission, exact reference rebuilding, register-resource
  retention, recovered structured lifting, unified classpath views, every
  supported JVM and Dalvik LLIL operation, encoding-alias normalization, stale
  provenance rejection, and exact complete-body LLIL round trips. Cover MLIL
  construction and rejection, typed operation signatures, provenance expansion
  and fusion, exceptional pre-state commits, constructor alias refinement,
  implicit DEX results and exceptions, dominance and SSA, both frontend
  lift/lower adapters, same-ISA LLIL-to-RTL-to-MLIL-to-RTL-to-LLIL relifting,
  and cross-ISA MLIL-to-target-RTL-to-LLIL-to-RTL-to-MLIL relifting through
  Cafe, including references, arrays, calls, and exceptional control flow.
  Cover MLIL definition/use,
  liveness, constant, expression, copy, semantic-alias, dead-code, and
  structured-control analyses on payload-bearing graphs. Compile generated Java source with
  `javac` and execute fixtures covering arithmetic, branches, loops, objects,
  fields, calls, casts, boolean coercion, arrays, switches, ordered exception
  handlers, source maps, diagnostics, and conservative unsupported stubs.
  Cover hierarchy-proven cast omission and archive-wide method-exception
  decisions without treating missing classpath declarations as proof.
  Cover Cafe CLI parsing, effective multi-release JAR selection, single-reader
  aggregate and bounded concurrent decompilation, deterministic largest-first
  work scheduling, safe package-qualified output, overwrite and collision
  policy, deterministic diagnostics, partial-failure status, and malformed
  independent members. Verify indexed generated offsets against literal
  indentation across empty, multiline, and trailing-newline fragments.
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
