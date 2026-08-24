# JVM class-file corpus

These fixtures exercise lossless class parsing and assembly across class-file
versions and compiler families. Generated `.class` files and the modular JAR
are intentionally committed so the test suite does not require a JDK or a
network connection.

## OpenJDK javac fixtures

The checked-in javac fixtures were generated with OpenJDK 23.0.2:

```powershell
javac --release 8  -g -parameters -d classes/java8  src/legacy/LegacyCorpus.java
javac --release 11 -g -parameters -d classes/java11 src/legacy/LegacyCorpus.java
javac --release 17 -g -parameters -d classes/java17 src/corpus/FeatureCorpus.java
javac --release 23 -g -parameters -d classes/java23 src/corpus/FeatureCorpus.java
```

The modular fixture is produced in a temporary output directory and then
normalized by the JDK `jar` tool, which adds `ModulePackages` and
`ModuleMainClass` to `module-info.class`:

```powershell
$moduleOutput = Join-Path $env:TEMP cafe-module-classes
javac -g -parameters -d $moduleOutput (Get-ChildItem module-src -Recurse -Filter *.java)
jar --create --file module-corpus.jar --main-class corpus.module.Main -C $moduleOutput .
```

## Eclipse ECJ fixtures

`classes/ecj17` was generated from the same `FeatureCorpus.java` source with
Eclipse Compiler for Java 3.46.0 (`ecj-4.40.jar`, build
`v20260528-0407`) targeting Java 17:

```powershell
java -jar ecj-4.40.jar -17 -g -parameters -d classes/ecj17 src/corpus/FeatureCorpus.java
```

The compiler artifact came from the Eclipse 4.40 release at
`https://download.eclipse.org/eclipse/downloads/drops4/R-4.40-202606010713/ecj-4.40.jar`.
Its SHA-512 digest is:

```text
0b2c799e20dbdd074272faf2aed454186f0ad25759c09963350bceea8e9bec0ad9a04693115401cab6a1c9122b94aba6d79d741eef2e003a4149b2fe00f6f158
```
