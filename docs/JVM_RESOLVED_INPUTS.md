# Externally resolved Java compilation inputs

Excession can consume a classpath already resolved in a **trusted Maven or
Gradle workflow**, without running Maven, Gradle, project plugins, build
scripts, or dependency resolution itself. Neither build system's metadata
is intrinsically trusted, and Excession does not claim to authenticate
which build tool produced it.

Provide a JSON file external to the source checkout:

```json
{
  "schema": "excession.jvm-resolved-inputs.v1",
  "provider_claim": "maven",
  "source_commit": "40-character-lowercase-git-commit",
  "source_tree": "40-character-lowercase-git-tree",
  "source_prefix": "src/main/java",
  "sources": [
    "src/main/java/org/example/Helper.java",
    "src/main/java/org/example/Service.java"
  ],
  "classpath": [
    {
      "path": "/absolute/canonical/path/to/resolved-library.jar",
      "sha256": "64-character-lowercase-sha256",
      "size_bytes": 10000
    }
  ],
  "release": "17"
}
```

The placeholder identities must be replaced with actual values. The
`provider_claim` may be `"maven"` or `"gradle"`. It is **not evidence
that either tool ran**. An upstream build or trusted CI export must
provide the resolved JAR paths, classpath order, source set, release and
hashes. Excession does not guess them from `pom.xml` or `build.gradle`.

```sh
excession analyze native --language java \
  --source src/main/java/org/example/Service.java \
  --source-prefix src/main/java \
  --classpath-manifest /tmp/jvm-inputs.json \
  --trusted-local --json > /tmp/java-configured.json

excession evidence native-import \
  --repo . --rev HEAD \
  --report /tmp/java-configured.json \
  --db /tmp/excession.sqlite

excession evidence native-store-audit \
  --repo . --rev HEAD \
  --snapshot <snapshot_id> \
  --db /tmp/excession.sqlite
```

## What is checked

- Exact Git commit/tree, selected tracked Java files and source-prefix
  inventory. The sources must be lexically sorted and match the complete
  tracked Java set within the selected prefix.
- Classpath order and every absolute, canonical, regular JAR file.
  No symlinks. SHA-256 and file size are checked on input, on a private
  staged compiler copy, after analysis, and on evidence import/replay.
- At most **128 Java sources** totaling **4 MiB**, and **16 JARs**
  (16 MiB each, 64 MiB total). External dependencies must be available
  locally during later audit/replay.
- JDK compiler symbol resolution under explicit `-proc:none`,
  `-implicit:none`, empty sourcepath, and optional validated `--release`.
  No compilation outputs are emitted by the helper.

The raw receipt preserves the claimed provider, source selection, compiler
version, JAR identities and supported release. Java source-to-source
references remain typed `NATIVE_JAVA_SYMBOL_REFERENCE` observations.
Classpath bytes are authenticated but **build identity, dependency
resolution, compiler invocation parity, and provenance are not established**.
Missing or altered JARs fail closed, including when a prior report is
selected by `analyze`, `impact`, or an agent.

## Unsupported configurations

The workflow does not support project-local `.class` directories, a JPMS
module path, multi-module build-order resolution, generated source sets,
annotation processing, arbitrary compiler arguments, Kotlin or Scala, or
complete Maven/Gradle build equivalence. It cannot prove that the supplied
JAR list is complete or that the build ran. Obtain a configured compiler
receipt from a trusted existing build environment if those properties
are required. The compiler executes in trusted-local mode, **not a sandbox**.
