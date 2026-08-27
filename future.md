# Future work

Cafe is Java-specific. Its shared model and disassembly layers bridge binary
formats used by the Java ecosystem; they are not a universal binary-analysis
framework.

Cafe has two primary instruction families:

- JVM bytecode stored in `.class` files;
- Android DEX bytecode, including its CompactDex storage form.

Both families now have parsing, assembly, executable analysis, ISA-specific
reversible LLIL and RTL, bidirectional LLIL → RTL → shared typed MLIL
adaptation, corpus, archive, provenance, shared adapters, unified classpath, and
verified Program-to-native emission. They intentionally do not share one
low-level instruction model. Higher-level data-flow and structured-control
analysis converge above MLIL, and JVM class files can be recovered as Java
source through a focused decompiler and a first-party JAR-to-source command.
Whole-artifact DEX-to-class/JAR translation remains a separate future boundary.

The shared CFG boundary now retains stable parallel edge identities and typed
caller payloads for switch arms, ordered handlers, exact instruction throw
sites, and legacy continuation call sites. Borrowed edge-filtered
views, edge-sensitive pre/post-state dataflow, semantic validation hooks, and
explicit rewrite identity maps are supplied by cfglib. Shared graphs expose its
generic exception model while retaining exact native exception metadata on
stable edge payloads. They also register exact protected block sets and ordered
handler entries/kinds as cfglib regions, explicitly marking handler-body
extents unknown when JVM or DEX metadata does not encode them. A separate
recovered exception model maps native table entries to cfglib handler
identities, retains stable exceptional edge and throw-site evidence, and
conservatively reports handler-exclusive blocks, shared boundaries, and
ambiguities. Exact catch descriptors are keyed by stable cfglib handler
identities. A derived structured-lifting bridge promotes only complete,
nonambiguous, nonoverlapping recovered handler bodies on a cloned CFG; it never
changes canonical unknown extents or invents source-level `finally`. Native
opcode throwability suppresses impossible exceptional edges while unknown
opcodes remain conservative. The seeded fallible edge solver now drives JVM
verification-frame and DEX register-frame analysis directly while preserving
native errors and source locations. These are infrastructure for analysis and
later transformations and cross-ISA function lowering; they do not implement
whole-artifact DEX-to-JVM translation policy.

## Development principles

- Keep `cafe` as the only library consumer dependency. Re-export every focused
  crate and public capability through Cafe in the same change. First-party
  applications must consume the same facade rather than implementation crates.
- Add a format only for a concrete consumer and representative fixtures.
- Do not add speculative `BinaryFormat` variants before their frontend exists.
- Implement parsing and assembly together wherever a format is editable.
- Preserve unknown metadata and original string data so unchanged artifacts
  can round-trip exactly.
- Retain native indices, addresses, flags, signatures, references, exception
  metadata, and exact source identities for downstream consumers.
- Lift executable bodies into `disassembler` and owned definitions into
  `program`; format-specific structures remain in focused frontend crates.
- Build and verify ordinary, branch, switch, exceptional, and legacy control
  flow.
- Keep format-qualified source maps and structured diagnostics in the neutral
  disassembly boundary; transformation policy belongs in a focused consumer.

## Completed JVM hardening baseline

The `java` crate now provides:

- lossless class-file parsing and assembly, including unknown attributes and
  exact modified UTF-8/UTF-16 data;
- complete JVM bytecode decoding and encoding, typed reference resolution,
  malformed-input checks, symbolic layout, branch relaxation, exact
  stack/local analysis, and deterministic stack-map generation;
- JVM-specific LLIL with normalized stack-machine operations, exact native
  encoding provenance, checked instruction and complete-body lift/lower paths,
  and lossless exception, stack-map, and debugging metadata retention;
- independently testable attributes for stack maps, bootstrap methods,
  dynamic constants, modules, records, nests, sealed types, annotations, and
  debugging metadata;
- one-reader JAR traversal, validation, editing, signature policy, and
  multi-release selection;
- read-only JMOD and JIMAGE ingestion, including compressed JIMAGE resources;
- deterministic, non-fail-fast corpus validation with artifact, entry, class,
  method, byte-offset, and processing-stage diagnostics;
- explicit metadata-only and executable-body adapter policies for shared
  disassembly and Program;
- verified canonical Program-to-class-file emission with structured constant
  re-interning, instruction reconstruction, frame propagation, stack-map
  generation, assembly validation, and parse-back coverage.

JAR, JMOD, and JIMAGE are container boundaries around JVM class files. They do
not introduce additional instruction semantics.

## Completed DEX and Android hardening baseline

The `dex` crate now provides:

- lossless standard DEX parsing and assembly for the supported 035 through 041
  family, including first-class multi-header DEX 041 containers;
- a complete standard Dalvik instruction codec, typed executable semantics,
  verified normal and exceptional control flow, and fixed-point register
  analysis;
- Dalvik-specific LLIL with typed register definitions and uses, normalized
  opcode encodings, explicit payload data, exact native provenance, checked
  instruction and code-item lift/lower paths, and lossless handler/debug/frame
  metadata retention;
- stable symbolic interning and builders for strings, types, prototypes,
  fields, and methods without manual table-order bookkeeping;
- typed CompactDex 001 headers, split main/shared sections, offset tables,
  method inventories, debug offsets, and code-item decoding and encoding;
- one-reader APK and Android App Bundle traversal with deterministic multidex
  or module provenance and raw-byte visitors for non-fail-fast consumers;
- APK rewrite reports that distinguish blocking conditions from reproducibility
  losses and require explicit signature-material policy;
- deterministic, non-fail-fast corpus validation across standalone DEX,
  multi-header DEX 041, APK, and AAB artifacts;
- cross-format resolution coverage proving that equivalent JVM and DEX
  declarations retain separate format-qualified identities;
- verified canonical Program-to-DEX emission with deterministic identifier
  rebuilding, exact retained register resources, instruction and handler
  reconstruction, native validation, and parse-back coverage.

Exact pristine output, contextual malformed-input errors, and verified control
flow remain release gates.

## Completed ISA-specific LLIL boundary

The JVM and Dalvik LLILs live in their owning frontends and deliberately model
different abstract machines:

- `java::llil` retains implicit JVM operand-stack behavior while normalizing
  immediate, local-slot, branch-width, switch-shape, invocation, and `wide`
  encoding variants;
- `dex::llil` exposes typed register definitions and uses, implicit result
  behavior, throwability, references, literals, targets, and non-executable
  payloads while normalizing Dalvik width, range, two-address, and literal
  opcode variants.

Every LLIL instruction carries normalized semantics separately from exact
native encoding provenance. Checked lowering requires those views to agree and
then passes the reconstructed stream through the native codec. Complete-body
adapters additionally retain and revalidate JVM exception/nested attributes or
DEX register resources, ordered handlers, debug programs, and source offsets.
Tests cover every supported native opcode, exact wide and payload encodings,
encoding-alias normalization, stale-pair rejection, and whole-body metadata
round trips.

There is no direct JVM-LLIL-to-Dalvik-LLIL conversion. Each LLIL first lifts
into its owning frontend's RTL, then cfglib raises typed def-use webs into
Cafe's Java-managed dialect. Stack and register representations converge only
in the explicit MLIL value model, without leaking one ISA's mechanics into the
other frontend or Java-specific semantics into cfglib.

## Completed frontend-owned RTL boundary

Cfglib now models RTL and MLIL as genuinely distinct dialects. One semantic
MLIL dialect can bridge any number of target RTL dialects whose native storage
types, constraints, operators, effects, and edge payloads differ. The bridge
provides:

- fallible exact edge translation after statement/instruction rewrite maps are
  complete, including stable exceptional throw-site identities;
- checked signature and structured exception-region translation, including
  block expansions introduced by normal-flow commit continuations;
- deterministic many-to-many statement/instruction provenance in both
  directions;
- explicit source-native provenance translation, so generated target storage
  and synthetic temporaries do not masquerade as source locations.

`java::rtl` directly decodes verified JVM LLIL into source locals, operand-stack
positions, and synthetic JVM temporaries. `dex::rtl` directly decodes verified
Dalvik LLIL into source registers, implicit result/exception channels, and
synthetic Dalvik temporaries. Both use their native verifier state as RTL
constraints and raise into the same canonical `mlil::JavaDialect`. In reverse,
MLIL lowers into the selected target RTL with fresh target allocation before
the owning frontend emits canonical LLIL and native encoding. Tests cover both
same-ISA round trips and JVM ↔ Dalvik retargeting through every level.

## Completed shared MLIL boundary

Cfglib now provides generic RTL and MLIL functions, instructions, stable identities,
typed variable occurrences, checked construction, many-to-many source
provenance, structural verification, and reusable analysis integration through
consumer-defined dialect contracts. Cafe's `mlil` crate now provides the
Java-managed dialect and preserves the concrete public API. Together they
provide:

- typed mutable variables for parameters, locals, temporaries, conditions, and
  delivered exceptions, with format-qualified JVM local/stack and DEX
  register/result/exception provenance;
- one Java-ecosystem operation vocabulary for constants, copies, primitive
  arithmetic, conversion and comparison, branches, switches, returns, throws,
  arrays, fields, calls, allocation, casts, monitors, caught exceptions, and
  explicit intrinsics;
- canonical semantic operand ordering independent of native JVM stack or DEX
  encoding order;
- one canonical JVM-compatible descriptor spelling for exact object and array
  value types, plus an explicit Dalvik zero/null lattice value;
- cfglib-owned control flow and MLIL storage with a synthetic root, stable
  typed edge payloads, exact native switch keys, ordered catches, protected
  ranges, and throw-site instruction identities;
- exception-precise normal-flow commit blocks, so definitions produced by a
  protected throwing instruction are invisible along its exceptional edges;
- synthetic handler landings that materialize delivered exceptions without
  inventing native handler-body extents or source-level `finally`;
- deterministic many-to-many native provenance for instruction expansion,
  synthetic semantics, and DEX payload fusion;
- strict structural, typing, terminator, edge, exception, identity, and
  provenance verification, followed by dominance and SSA construction;
- definition/use chains, liveness, forward and sparse constants, block-local
  expression recovery, copy-propagated views, effect-aware dead-code analysis
  and elimination, and conservative cfglib structured-control recovery over
  the canonical payload-bearing graph.

The JVM adapter consumes verified frame states and preserves constructor alias
initialization through explicit type refinement. The DEX adapter consumes
verified register states, makes implicit invocation/filled-array results and
`move-exception` state explicit, and excludes payload records from executable
blocks while retaining their provenance. Both adapters reject unsupported or
inconsistent semantics at overload-qualified native locations.

## Completed shared transformation prerequisites

The neutral layers and focused frontends now provide:

- exact structured reference symbols for numeric constants, Java UTF-16 text,
  types, fields, methods, and method prototypes, while retaining original
  indices only as provenance;
- exact DEX register, incoming, and outgoing resource widths on shared bodies;
- reversible, ISA-specific JVM and Dalvik LLIL/RTL boundaries with exact native
  provenance and complete-body metadata validation;
- a verified shared MLIL with frontend-owned JVM and Dalvik LLIL-to-RTL lifting,
  cfglib RTL raising, and origin-independent lowering through either target RTL
  to target LLIL, exact exceptional
  pre-state, format-qualified provenance, dominance, and SSA;
- a format-neutral `ModuleEmitter` contract with verified JVM and DEX native
  backends;
- a unified `ClasspathHierarchy` that ingests native files, Program modules,
  and supported JVM/Android containers, merges equivalent declarations, and
  exposes borrowed JVM and DEX analysis views;
- exact cfglib handler-type identities, native opcode throwability, recovered
  exception ownership evidence, and a conservative derived structured-lifting
  path;
- reusable MLIL data-flow, expression, dead-code, and control-structure
  analyses that retain caller-owned edge payloads.

These capabilities support native editing, shared semantic analysis,
same-family rebuilds, and cross-ISA function retargeting. MLIL-to-LLIL lowering
is canonical rather than byte-identical: the original LLIL encoding sidecar
remains the exact replay path, while the selected target backend generates a
fresh valid layout.

## Completed frontend-owned RTL and LLIL lowering

`java::rtl` and `dex::rtl` first lower verified MLIL into fresh target-specific
RTL. Their existing target emitters then lower canonical semantics into LLIL
without putting target mechanics in the neutral `mlil` crate. Source format is
provenance, not lowering eligibility:

- JVM lowering assigns semantic variables to locals, schedules stack operands,
  selects canonical bytecodes, uses source-pool or caller-provided reference
  resolution, lays out symbolic control flow, and reconstructs exact throw-site
  ranges and ordered catches;
- Dalvik lowering allocates parameters and temporaries into a fresh register
  frame, reserves contiguous scratch/range operands, selects canonical opcodes,
  lays out long branches and aligned switch/array payloads, resolves identifier
  tables, and reconstructs ordered try handlers;
- MLIL array types use exact semantic descriptors, and array initialization
  uses typed constants rather than JVM allocation-form flags or raw DEX payload
  bytes; each backend selects or expands its native form, including wide and
  oversized Dalvik initialized arrays;
- JVM multi-dimensional allocation is legalized into checked nested Dalvik
  allocation loops with Java-compatible negative-dimension exception order,
  while primitive DEX class literals become the corresponding JVM wrapper
  `TYPE` field;
- calls retain semantic dispatch plus separate declared-target and effective
  descriptors for signature-polymorphic invocation;
- implementation-defined intrinsics require explicit target policies; policy
  expansions are verified, may not hide control flow, and may not erase a
  potentially throwing native interval;
- both return deterministic source maps from original native provenance to
  generated ranges, reject only invalid or target-unencodable semantics, omit
  stale offset-based metadata, validate the generated LLIL, and pass
  LLIL-to-MLIL-to-either-LLIL-to-MLIL tests for data flow, references,
  null/zero legalization, calls, arrays, control flow, and exceptional flow.

The default JVM lowerer symbolically interns references into a target constant
pool. The default Dalvik lowerer symbolically resolves them against explicit
target identifier tables, which can be prepared with `DexBuilder`. Separate
`lower_body_from_source` entry points reuse checked native indices only when the
caller intentionally supplies the original source tables. Recursive bootstrap,
call-site, and method-handle metadata requires a caller-provided target resolver
with access to the destination artifact's tables. This is deliberately target
linkage policy rather than source-ISA mechanics or a reason to make MLIL
recursive constant-pool storage.

Function-level retargeting is deliberately checked rather than approximate.
Every currently modeled cross-ISA semantic operation either has a verified
target form, uses explicit caller linkage or intrinsic policy, or returns a
structured target-encoding error. Stack, register, constant-pool, identifier,
and payload mechanics remain in their owning lowerers rather than leaking into
MLIL.

## Completed generic analysis and Java source decompilation

The generic `cfglib::mlil` API exposes reusable definition/use chains, liveness,
forward and sparse constants, expression trees, copy and pairwise semantic-alias
propagation, effect-aware dead-code reporting and elimination, and
structured-control recovery. Cafe's MLIL dialect supplies Java-managed folding,
effects, call targets, edge meaning, and semantic verification. Cfglib's
solvers and AST lifter accept caller-owned edge payloads directly, and natural
loops are detected by dominance even when a frontend correctly retains the
native edge as an ordinary jump. Memory reads, writes, allocation, calls,
synchronization, throwing operations, and control flow remain observable during
dead-code elimination; Cafe does not speculate about heap aliases to erase
source-visible operations.

The focused `decompiler` crate, publicly available as `cafe::decompiler`, now
provides:

- parsed-class and raw-byte JVM class-file entry points plus a hierarchy-aware
  variant for strict frame merging;
- class-family entry points that aggregate metadata-proven named member classes
  recursively into their enclosing compilation unit;
- per-method lifting through verified MLIL and reusable analysis over the same
  exact payload-bearing graph;
- a cfglib-owned named, ordered, fallible pass pipeline with change reports,
  plus dependency-safe decompiler pass profiles whose recommended default
  normalizes one derived MLIL presentation graph without mutating canonical
  verified MLIL;
- Java declarations for ordinary classes and interfaces, fields, methods,
  constructors, static initializers, parameters, constants, and declared
  exceptions;
- structured Java branches and natural loops when cfglib proves a reducible
  representation, with an exact Java-valid state machine for switches,
  irreducible flow, and ordered exception dispatch;
- typed rendering for constants, arithmetic, conversions, comparisons,
  arrays, fields, calls, allocation and constructor aliases, casts,
  `instanceof`, returns, throws, and caught exceptions;
- deterministic diagnostics and UTF-8 generated spans mapped through stable
  MLIL instruction identities to all contributing native bytecode ranges;
- a single-decode JVM lifting path shared by LLIL, frame analysis, and RTL,
  indexed generated-span translation, and deterministic largest-unit-first JAR
  scheduling so unusually large methods do not create a serial completion tail;
- conservative throwing stubs when a method cannot be represented, including
  Java-legal static-initializer stubs, rather than guessed source semantics;
- canonical `InnerClasses` type names and exact member modifiers, generic class,
  field, and method declarations from `Signature`, omission of
  source-inexpressible bridge duplicates, direct unchanged receiver/parameter
  names, guarded removal of runtime-identical constructor refinements,
  hierarchy-proven cast omission, archive-wide method-exception decisions,
  implicit empty constructors and trailing void returns, and diagnosed removal
  of `final` where bytecode initialization has not yet been reconstructed as
  Java definite assignment;
- compile-back and execution coverage through `javac` for arithmetic,
  branching, natural loops, objects, calls, fields, casts, boolean coercions,
  arrays, switches, and ordered handlers.

Dynamic bootstrap reconstruction and synchronized-region formation still
produce method diagnostics and stubs. Annotation, enum, and record declarations
are explicitly diagnosed approximations, `module-info` is a separate source
artifact rather than a class declaration, and local or anonymous class
placement still requires method-level recovery. These are source-presentation
limits, not missing JVM parsing, MLIL semantics, or cross-ISA lowering support.

## Completed command-line JAR decompilation

The `cafe-cli` application provides `cafe decompile jar`. It consumes only the
public `cafe` facade and owns Clap syntax, filesystem policy, human-readable
reporting, and process exit status. The command selects an effective
multi-release view, visits selected class payloads through one ZIP reader,
builds a JAR-wide class hierarchy and exact method-exception catalog, and writes
deterministic package-qualified Java source paths using the decompiler's
identifier escaping.
Named member classes are grouped recursively under their metadata-declared
top-level owner, so one Java compilation unit owns one output path. Source
recovery uses a bounded automatic worker count with an explicit `--jobs`
override while diagnostic ordering and output-collision decisions remain
deterministic.

Malformed or unsupported independent classes are retained as aggregate failures
while later classes continue. Existing files require explicit `--force`, output
paths cannot traverse beyond the requested directory, and symbolic-link targets
are rejected. Structured decompiler diagnostics remain visible, and error-level
recovery diagnostics or class failures produce a nonzero status without deleting
successfully generated source. The console sample is bounded to 100 diagnostics;
`--all-diagnostics` emits the complete deterministic report. Module descriptors
are reported as skipped until module-declaration source recovery exists. This
command does not perform DEX to JVM translation or packaging.

## Remaining whole-artifact boundary

Cafe now has cross-ISA function-body retargeting: typed DEX/JVM analysis lifts
through frontend RTL to verified shared MLIL, then either frontend lowers the
same function through its target RTL into independently verified LLIL. It
intentionally does not yet implement
whole-program class and member synthesis, runtime-library mapping, manifest and
resource policy, dex2jar-equivalent packaging, a DEX-to-JAR workflow, or a
DEX-oriented source frontend.

If a whole-artifact DEX-to-JVM workflow is added later, keep its coordination
and policy in another focused feature crate over Program, MLIL, and the JVM
backend. It must define class/module mapping, target linkage construction,
Android-runtime substitutions, debug-information policy, output packaging, and
unsupported semantics. It may not move DEX/APK details into `java`,
JVM/class-file details into `dex`, or frontend semantics into the neutral
`disassembler`, `mlil`, and `program` layers. Unsupported cases must produce
explicit diagnostics with native source locations rather than silent
approximations.

## Completed JNI boundary hardening

The `jni` crate now provides:

- exact UTF-16 descriptor-to-ABI mapping and canonical short/long JNI symbols;
- native-only overload planning and safe opaque `RegisterNatives` resolution;
- Java target-release selection before multi-release JAR scanning;
- provenance-retaining reports for class files, JARs, DEX, APK, and AAB;
- policy-controlled portable C header rendering;
- Java 24-and-later module native-access requirement reporting.

Native library parsing, platform calling-convention implementation, process
loading, and machine-code analysis remain outside this workspace.

## Completed Android runtime boundary

The `art` crate keeps runtime container state outside canonical DEX and exposes
it through `cafe::art`. It provides:

- validated VDEX 009, 012, 020, 021, and sectioned 027 layouts;
- standard DEX restoration from quickening metadata and explicit CompactDex
  split-section retention;
- ODEX 036 parsing, exact preservation, checksums, opaque dependency and
  optimization payloads, plus explicit same-width canonical patches;
- stable OAT metadata discovery in direct or ELF-contained artifacts while
  leaving native instructions and architecture-specific state opaque;
- canonical quick-opcode restoration before standard DEX reaches shared
  disassembly or Program adapters.

APK, AAB, VDEX, ODEX, and OAT are containers or optimized encodings, not new
shared instruction sets.

## Conditional Java Card extension

If a real smart-card consumer appears, add a sibling `crates/javacard`
frontend for CAP components and Java Card VM bytecode. Do not place CAP parsing
inside `java` or `dex`: its packaging, instruction encoding, linking model, and
runtime constraints are distinct. Expose it as `cafe::javacard`.

Until that consumer exists, Java Card is a documented extension point, not
planned implementation work.

## Non-goals

Cafe does not plan to support:

- .NET CIL;
- WebAssembly;
- Python, Lua, JavaScript, or other scripting-VM bytecode;
- x86, Arm, RISC-V, or another native instruction set;
- JIT compiler intermediate representations or generated machine code;
- source-language parsing for Java, Kotlin, Scala, Groovy, or Clojure.

Those source languages already converge on JVM bytecode or DEX for Cafe's
purposes. Native code and unrelated virtual machines require different operand,
memory, relocation, calling-convention, and runtime models and belong in
separate projects.

## Completion standard for a new frontend

A frontend is ready when all of the following are true:

1. Its supported artifact versions and rejection behavior are explicit.
2. Parsing and assembly cover the same editable structures and instructions.
3. Unchanged fixtures round-trip exactly where the format permits it.
4. Malformed offsets, indices, lengths, descriptors, and control-flow targets
   fail with contextual errors rather than panics.
5. Every executable body lifts into verified shared control flow.
6. Its instruction family has an ISA-specific semantic LLIL with checked,
   exact native and complete-body round trips.
7. Its verified native analysis and LLIL lift into an ISA-owned RTL, cfglib can
   raise that RTL into shared MLIL with exact control-flow, exception, type, and
   provenance semantics, and frontend-owned lowering can generate independently
   verified target RTL and LLIL without using source format as eligibility.
8. Metadata-only loading avoids decoding executable bodies.
9. Program identities remain format-qualified and overload-qualified.
10. Archive discovery and traversal are deterministic and retain exact origins.
11. Public APIs are documented, source files remain below 1,000 lines, and the
   complete repository verification gate passes.
12. The frontend and its capabilities are reachable through `cafe` without a
    second direct dependency.
