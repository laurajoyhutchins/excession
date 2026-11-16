# Configured compiler checks

Excession runs the project's native checker. It does not introduce another compiler or turn source indexing into proof of compilation.

A checked repository must be clean and at the selected Git HEAD. Execution is opt-in. Native compilers, build scripts and typechecker plugins may execute local code; this command is not a sandbox.

## Running and auditing

Store evidence outside the source checkout.

~~~sh
excession check run --repo . --language rust --config Cargo.toml --db /tmp/checks.sqlite --trusted-local true
excession check run --repo . --language typescript --config tsconfig.json --db /tmp/checks.sqlite --trusted-local true
excession check run --repo . --language python --config mypy.ini --source-prefix src --db /tmp/checks.sqlite --trusted-local true
excession check audit --repo . --rev HEAD --db /tmp/checks.sqlite --snapshot SNAPSHOT_ID
~~~

The optional --tool /absolute/path selects a known local checker executable. No installation or downloads happen. Every tool launches without a shell and has an execution deadline and bounded logs.

| Language | Checker invocation | Output policy |
| --- | --- | --- |
| Rust | cargo check --all-targets --frozen on the selected Cargo.toml | Offline; external scratch CARGO_TARGET_DIR |
| TypeScript | tsc --project CONFIG --noEmit --incremental --tsBuildInfoFile SCRATCH | No emitted JavaScript |
| Python | mypy --config-file CONFIG --no-incremental --cache-dir SCRATCH TARGET | Scope explicitly selected |

Exit zero returns PASS; a nonzero exit returns FAIL with a persisted receipt, JSON on stdout and exit status 1. Unsupported language/configuration, dirty checkout, missing tool, timeout or excessive output is a refusal instead of a fabricated check.

## Evidence semantics

The excession.compiler-check.v1 receipt contains Git commit/tree/configuration blob and scope, checker version and launcher digest, process exit code and stdout/stderr hashes, bounded diagnostic previews and the explicit coverage limits. Original bytes are retained in the existing SQLite evidence store. Audits replay them against the exact original Git configuration without rerunning the compiler.

The launcher digest does not cover its complete dependency closure. Compiler-selected source membership and dependencies may be external or generated. Other project configurations, build features and workspaces are NOT_CHECKED. Local execution is TRUSTED_LOCAL_NOT_ATTESTED, not an independent remote execution attestation.

The pre-existing artifact build-record contract continues to document artifact bytes and declared inputs, without claiming a build ran. The configured checks do not yield distributable artifacts. Build provenance, REA, and artifact correctness remain independently required. Neither a successful typecheck nor an immutable receipt establishes runtime equivalence or universal safety.

The first version supports a single Cargo manifest, a single tsconfig.json project, or one selected mypy configuration and Python source scope. Build-script behavior, conditional compilation, TypeScript references/plugins and Python dynamic behavior can create additional verification obligations. Run untrusted code in a separately isolated worker.

## Git-declared configuration matrix

Use a tracked matrix to run multiple Cargo manifests, TypeScript projects and
Python source scopes without pretending a single check covers every target.

Create and commit a file named compiler-checks.json:

~~~json
{
  "schema": "excession.compiler-matrix-config.v1",
  "checks": [
    {
      "language": "rust",
      "configuration": "crates/core/Cargo.toml",
      "source_prefix": null,
      "declared_inputs": ["crates/core/Cargo.lock"]
    },
    {
      "language": "typescript",
      "configuration": "web/tsconfig.json",
      "source_prefix": null,
      "declared_inputs": ["web/package-lock.json"]
    },
    {
      "language": "python",
      "configuration": "mypy.ini",
      "source_prefix": "src",
      "declared_inputs": ["requirements/typecheck.txt"]
    }
  ]
}
~~~

Run it with native checkers already installed and with a clean checkout:

~~~sh
excession check matrix --repo . --manifest compiler-checks.json \
  --db /tmp/excession-checks.sqlite --trusted-local true
excession check matrix-audit --repo . --rev HEAD \
  --db /tmp/excession-checks.sqlite --snapshot SNAPSHOT_ID
~~~

Use --rust-tool, --typescript-tool and/or --python-tool with absolute
executable paths when the checker is not on PATH. All configurations are
preflighted against their Git blobs before any checker executes. The matrix
then runs every declared check, including checks after a diagnostic failure.
Each check has its own immutable receipt and snapshot. The matrix stores a
separate aggregate receipt and audits all children against their original Git
configurations, declared input identities, results, and hashes.

A matrix PASS means every listed check returned exit zero. Missing dependency
evidence or external dependencies do not silently become a success claim.
Checks are user-declared, not automatically exhaustive. A lockfile in
declared_inputs is a Git-authenticated declaration, not proof the compiler
consumed it, fetched the expected packages, verified generated declarations,
or enumerated transitive dependencies.

The report records USER_DECLARATION_NOT_EXHAUSTIVE,
GIT_AUTHENTICATED_NOT_COMPILER_MEMBERSHIP and dependency_closure:
NOT_ESTABLISHED. Execution remains TRUSTED_LOCAL_NOT_ATTESTED.
A local SQLite audit cannot independently attest a remote executor; a
separately signed and verified execution chain is required to upgrade that
authority. External inputs, project references, feature flags, native
extensions and runtime behavior remain distinct obligations.

Limits: 1 to 16 entries and at most 32 declared inputs per entry. Each
input/configuration must be a regular tracked Git file and match the clean
working tree. Missing or unsupported inputs refuse before execution. A
missing checker partway through can leave independently auditable child
receipts, but never an aggregate matrix success.
