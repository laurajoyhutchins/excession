# Java compiler source evidence

The `analyze native --language java` producer uses the JDK compiler API
(`JavacTask.parse` and `JavacTask.analyze`) to resolve declarations
between tracked Java source files. The Java adapter does **not** parse Java
using a custom grammar or infer dependencies from import strings.

```sh
excession analyze native \
  --language java \
  --source src/main/java/org/example/Service.java \
  --source-prefix src/main/java \
  --trusted-local --json > /tmp/java-native.json

excession evidence native-import \
  --repo . --rev HEAD --report /tmp/java-native.json \
  --db /tmp/excession.sqlite

excession analyze . --db /tmp/excession.sqlite --json
excession impact . --db /tmp/excession.sqlite --json
excession agent context --repo . --rev HEAD \
  --focus src/main/java/org/example/Helper.java \
  --db /tmp/excession.sqlite
```

## Scope and authority

A Java analysis reads **only** committed regular `.java` files within
`--source-prefix` (or all tracked Java files if omitted). It verifies their
checkout bytes against Git before and after the compiler executes, requires
a clean HEAD, and refuses more than 128 files or 4 MiB of source.

The adapter invokes only the installed JDK source launcher on Excession's
fixed helper and uses compiler APIs to resolve source symbols. The compiler
is configured with empty class and source paths, `-proc:none`, and
`-implicit:none`. It does not run Maven, Gradle, annotation processors,
generated code, user classes, or build scripts. JDK library references
are not source-to-source edges. It refuses missing third-party dependencies,
and cannot reproduce a project-specific compiler configuration.

The native report has a Git-bound `excession.native-analysis.v1` envelope
and a nested `excession.java-native.v1` report containing compiler-resolved
source-to-source references. Each endpoint carries exact Git commit, tree,
path, blob and mode. Retained `NATIVE_JAVA_SYMBOL_REFERENCE` observations
identify the dependent source and referenced declaration. A changed
declaration appears as a configured observation in impact and agent
verification. These observations are **not** a complete call graph or
proof of build success; they are producer-supplied and unattested.

The existing immutable evidence snapshot format, source replay,
revision-pinned planning, and agent obligations are reused. No new graph
store is introduced. The raw native receipt retains the source set,
compiler version and specific limitations. Imported reports cannot
establish source accuracy solely by recomputing a report digest: Git
endpoints are independently validated.

## Explicitly unsupported

A Java project requiring a third-party classpath, annotation processors,
Kotlin/Scala mixed compilation, Gradle/Maven generated code, or module
workflows must use a separate configured native producer. Excession
refuses this narrower source-only workflow rather than guessing those
semantics. A trusted-local compiler invocation is **not sandboxed**.
