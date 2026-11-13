# Build provenance is core evidence

The native build-evidence workflow records exact Git input files, declared
toolchain and configuration digests, and the independently read SHA-256 and byte
count of an existing binary. All records use Excession's single Rust/SQLite
evidence authority, with shared Source and Artifact subjects.

Usage:

    excession evidence build-record --repo . --rev <exact-commit> \
      --artifact /path/app --db /path/evidence.sqlite \
      --sources src/main.rs,src/lib.rs --toolchain <sha256> \
      --configuration <sha256> --representation EXECUTABLE

    excession inspect build-provenance --db /path/evidence.sqlite \
      --repo . --rev <exact-commit> --artifact /path/app \
      --snapshot <snapshot-id>

The writer stores one immutable original receipt, source-identity observations,
and an artifact-bytes observation. The auditor independently repeats the Git
identity checks and SHA-256 of the artifact. Reconstructing the exact original
snapshot also detects forged source, output, recipe or SQL rows.

A declared input does NOT prove a compiler executed or created the artifact.
The result preserves source_artifact_relation=NOT_ESTABLISHED and
build_execution_verified=false. It never executes arbitrary build commands,
and records a recipe as untrusted until an independently verified isolated
build receipt can discharge that obligation.

The same Artifact subject identity is the join point for reverse-engineering
observations and build results. It does not replace native REA IDs or meanings.

This is the live JSON report format, validated against real Rust CLI output:
[build-provenance.v1.schema.json](../../contracts/evidence-core/build-provenance.v1.schema.json).
No redundant ODCS catalog or Python production implementation is introduced.
