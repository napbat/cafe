# Future work

Cafe is Java-specific. Its shared model and disassembly layers exist to bridge
instruction formats used by the Java ecosystem, not to become a universal
binary-analysis framework.

Cafe currently needs exactly two primary instruction families:

- JVM bytecode stored in `.class` files;
- Android DEX bytecode.

Both are implemented by the current `java` and `dex` frontends. No additional
instruction set is needed for the present scope. CompactDex is an alternate
Android encoding of DEX concepts rather than a new language target. Java Card
CAP bytecode is the only other distinct Java-specific instruction set worth
reserving architectural space for, and it remains demand-driven.

## Development principles

- Add a format only for a concrete consumer and representative input corpus.
- Do not add speculative `BinaryFormat` variants before their frontend exists.
- Implement parsing and assembly together. Every decoded structure and opcode
  needs a matching encoding path.
- Preserve unknown metadata and original string data so an unchanged artifact
  can round-trip exactly.
- Keep native indices, addresses, flags, signatures, references, and exception
  metadata available to downstream consumers.
- Lower executable bodies into `disassembler` and owned definitions into
  `cafe`; format-specific structures remain in their frontend crate.
- Build and verify control-flow graphs for ordinary, branch, switch,
  exceptional, and legacy control flow.

## 1. JVM coverage and hardening

Continue hardening the existing `java` crate before broadening the workspace:

- validate parsing, assembly, and method-bytecode round trips against class
  files produced by multiple Java releases and compilers;
- model class-file structures as typed APIs when consumers need them while
  retaining unknown attributes losslessly;
- keep stack maps, bootstrap methods, dynamic constants, modules, records,
  nests, sealed types, annotations, and debugging metadata testable as
  independent concepts;
- expand malformed-input coverage for constant-pool references, descriptors,
  attributes, bytecode targets, exception tables, and archive paths;
- add deterministic corpus reporting that identifies the exact artifact,
  class, method, and byte offset for every failure.

JAR remains the primary archive boundary. JMOD and JIMAGE ingestion can be
added when Cafe needs to inspect complete JDK distributions; they contain JVM
class files and therefore do not introduce another instruction set.

## 2. DEX and APK hardening

The `dex` crate now owns typed DEX files, the complete standard Dalvik
instruction codec, shared adapters, APK editing, deterministic multidex
provenance, and explicit signature-material policies. Continue hardening that
baseline with:

- representative DEX and APK corpora from multiple Android toolchain releases;
- differential instruction and file-format tests against Android's published
  format behavior;
- cross-format resolution tests between equivalent JVM and DEX definitions;
- ergonomic builders and interning APIs for creating nontrivial DEX files
  without manual table-order bookkeeping;
- a first-class multi-header parser and assembler for DEX version 041
  containers;
- stricter APK rewrite reporting for metadata that cannot be reproduced by the
  configured ZIP encoders;
- Android App Bundle discovery only when a concrete consumer needs it.

Keep exact pristine output, matching parse/assemble coverage, contextual
malformed-input errors, and verified control flow as release gates rather than
future aspirations.

## 3. JNI boundary hardening

The `jni` crate now provides the safe metadata boundary between Java native
declarations and their implementations. It preserves exact UTF-16 names,
parses JVM descriptors into typed JNI ABI signatures, implements canonical
short and long symbol mapping, models explicit registration keys, chooses
exports using native-only overload sets, and extracts declarations from JVM
class files, DEX files, and APK multidex sets.

Extend that boundary only for concrete native-integration consumers:

- add target-Java-version selection before scanning multi-release JARs;
- retain class-file, DEX, and APK provenance in aggregate binding reports;
- render portable C header declarations from the typed ABI model when a code
  generation consumer defines its naming and formatting policy;
- model resolved `RegisterNatives` tables when a consumer can supply reliable
  registration metadata;
- report module native-access requirements without trying to reproduce JVM
  library loading or class-loader state.

Native library parsing, platform calling-convention implementation, process
loading, and machine-code analysis remain outside this workspace. JNI metadata
does not introduce another instruction set.

## 4. Android runtime encodings and containers

Add these only when Cafe must inspect installed or runtime-produced Android
artifacts:

- CompactDex decoding and encoding can extend the DEX frontend while retaining
  an explicit source-format identity.
- APK and future Android App Bundle support are archive discovery and
  provenance, not additional instruction sets.
- VDEX, ODEX, and OAT are ART runtime containers or optimized artifacts. Keep
  their parsing and dequickening in a separate `crates/art` boundary rather
  than leaking ART state into the canonical DEX model.
- Any quickened instruction must be restored to a well-defined canonical form
  before it is lowered into shared disassembly.

## 5. Java Card

If a real smart-card consumer appears, add a sibling `crates/javacard`
frontend for CAP components and Java Card VM bytecode. Do not place CAP parsing
inside `java` or `dex`: its packaging, instruction encoding, linking model, and
runtime constraints are distinct.

Until that consumer exists, Java Card remains a documented extension point,
not planned implementation work.

## Containers are not instruction sets

The following may need readers, inventory APIs, or provenance models, but they
do not justify new shared instruction semantics by themselves:

- JAR, WAR, EAR, JMOD, and JIMAGE;
- APK and Android App Bundles;
- VDEX, ODEX, and OAT;
- native libraries reached through JNI.

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
memory, relocation, calling-convention, and runtime models and should live in
separate projects.

## Completion standard for a new frontend

A frontend is ready when all of the following are true:

1. Its supported artifact versions and rejection behavior are explicit.
2. Parsing and assembly cover the same structures and instructions.
3. Unchanged fixtures round-trip exactly where the format permits it.
4. Malformed offsets, indices, lengths, descriptors, and control-flow targets
   fail with contextual errors rather than panics.
5. Every executable body lowers into verified shared control flow.
6. Metadata-only loading avoids decoding executable bodies.
7. Cafe identities remain format-qualified and overload-qualified.
8. Archive discovery and traversal are deterministic and retain exact origins.
9. Public APIs are documented, source files remain below 1,000 lines, and the
   complete repository verification gate passes.
