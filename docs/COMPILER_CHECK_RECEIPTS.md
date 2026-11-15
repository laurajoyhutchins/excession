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
