# Future work

Cafe is Java-specific. Its shared model and disassembly layers bridge binary
formats used by the Java ecosystem; they are not a universal binary-analysis
framework.

Cafe has two primary instruction families:

- JVM bytecode stored in `.class` files;
- Android DEX bytecode, including its CompactDex storage form.

Both families now have the parsing, assembly, executable-analysis, corpus,
archive, provenance, and shared-adapter infrastructure needed by downstream
tools. The only planned cross-family capability not implemented here is the
explicit DEX-to-JVM transformation boundary described below.

## Development principles

- Keep `cafe` as the only consumer dependency. Re-export every focused crate
  and public capability through Cafe in the same change.
- Add a format only for a concrete consumer and representative fixtures.
- Do not add speculative `BinaryFormat` variants before their frontend exists.
- Implement parsing and assembly together wherever a format is editable.
- Preserve unknown metadata and original string data so unchanged artifacts
  can round-trip exactly.
- Retain native indices, addresses, flags, signatures, references, exception
  metadata, and exact source identities for downstream consumers.
- Lower executable bodies into `disassembler` and owned definitions into
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
- independently testable attributes for stack maps, bootstrap methods,
  dynamic constants, modules, records, nests, sealed types, annotations, and
  debugging metadata;
- one-reader JAR traversal, validation, editing, signature policy, and
  multi-release selection;
- read-only JMOD and JIMAGE ingestion, including compressed JIMAGE resources;
- deterministic, non-fail-fast corpus validation with artifact, entry, class,
  method, byte-offset, and processing-stage diagnostics;
- explicit metadata-only and executable-body adapter policies for shared
  disassembly and Program.

JAR, JMOD, and JIMAGE are container boundaries around JVM class files. They do
not introduce additional instruction semantics.

## Completed DEX and Android hardening baseline

The `dex` crate now provides:

- lossless standard DEX parsing and assembly for the supported 035 through 041
  family, including first-class multi-header DEX 041 containers;
- a complete standard Dalvik instruction codec, typed executable semantics,
  verified normal and exceptional control flow, and fixed-point register
  analysis;
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
  declarations retain separate format-qualified identities.

Exact pristine output, contextual malformed-input errors, and verified control
flow remain release gates.

## Remaining cross-format transformation boundary

Cafe has the reusable prerequisites for a DEX-to-JVM consumer: typed DEX
semantics and register frames, symbolic JVM layout, JVM verification frames and
stack maps, owned symbols, source maps, structured diagnostics, and
format-qualified Program identities. It intentionally does not implement a
DEX-to-JVM instruction translator, dex2jar equivalent, or DEX-to-JAR workflow.

If that feature is added later, put policy in a focused feature crate that
depends on both frontends. It must not move DEX/APK details into `java`,
JVM/class-file details into `dex`, or either frontend into the neutral
`disassembler` and `program` layers. Unsupported semantic cases must produce
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
5. Every executable body lowers into verified shared control flow.
6. Metadata-only loading avoids decoding executable bodies.
7. Program identities remain format-qualified and overload-qualified.
8. Archive discovery and traversal are deterministic and retain exact origins.
9. Public APIs are documented, source files remain below 1,000 lines, and the
   complete repository verification gate passes.
10. The frontend and its capabilities are reachable through `cafe` without a
    second direct dependency.
