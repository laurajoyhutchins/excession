# RelGit v0.14 additional same-function composition

New commands: `relgit compose-overlap SPEC_JSON DATABASE` and
`relgit compose-overlap-audit SPEC_JSON DATABASE`. Their native CPython
AST slot adapter is in `adapters/python_ast_slots.py`, the Rust authority
is in `src/overlap.rs`, and the complete offline reproduction is
`../scripts/reproduce_overlap.py`. See `../README.md` for result and scope.

---

# RelGit 0.11.1: Rust-owned core, language-native semantics

This is the next Rust implementation of RelGit's Git/SQLite engineering research system, built and tested against the original Python/JavaScript reference experiments. **Git objects remain the source authority. SQLite indexes derived relationships and explicitly scoped evidence.** The Rust kernel does not claim to infer Python or JavaScript semantics.

This release incorporates the previously verified v0.10 Git/index/integration core and ports the *language-neutral control plane* for physical source projections, mapped historical patch replay, contract/provenance indexing, source-identity impact, Python metadata projection orchestration, and workload-based cost decisions. The actual Python/JavaScript syntax and behavioral assertions remain in the native runtimes.

## v0.11.1: bounded local evidence carry (experimental)

`relgit carry-local ROOT [DATABASE]` is a **local process-supervised** mode for the exact
pinned h11 historical branch pair from the comparative trial. It requires the
known base, A and B commit hashes **before executing any Python test code**,
runs the two native regression groups concurrently under CPython 3.13 local
code monitoring, validates their actual process outcomes and interface traces,
performs independent Git/tree checks, and stores only the required evidence
relations in SQLite. The resulting guard matches v9/v11 detached carry for
the same Git history. This deliberately does **not** infer that arbitrary
agent source can be executed safely.

`relgit carry ROOT [DATABASE]` remains the detached unsigned-evidence path:
it independently reruns environment and interface probes. `relgit index`
continues to build the **full** language-dependent symbol/dependency index.
The local evidence mode indexes complete Git trees, module identities and
required claims, but skips parsing unrelated symbols/dependencies. Do not
query the evidence-only database as though it contained a full dependency
catalog.

This path does **not** accept remotely submitted unsigned claims, skip
independently required joint tests, automatically resolve overlapping edits,
or establish unrestricted Python compatibility. A trusted isolated executor
and authenticated evidence are prerequisites for untrusted multi-agent use.

## Responsibility boundary

| Operation | Owner | Rationale |
| --- | --- | --- |
| Git commit/tree/blob identities, file modes, path guards, and exact tree reconstruction | Rust | Language independent; Git authoritative |
| SQLite schema, bindings, transactions, Git/relational identities, integrity checks | Rust | Language independent; one catalog |
| Dependency/symbol catalog and queryable views | Rust orchestration | Native parsers supply syntax facts |
| Typed source moves, plan validation, compat-shim materialization | Rust | Git and file operations need one authority |
| Python import rewrites | CPython `ast` adapter | Language semantics and relative import syntax |
| JavaScript/TypeScript import rewrites | TypeScript compiler API adapter | `require`, import/export syntax and module metadata |
| Historical patch provenance, mapped patch scope, Git three-way merge | Rust + native Git | Fail closed; do not invent a new merge algorithm |
| Contract graph, source closures, bounded observations/derived claim records | Rust | Relations and claim status are explicit, not inferred from AST imports |
| h11 writer/reader contract, corpus, runtime and finite witness checker | Existing pinned Python sources | The original independent behavioral oracle is authoritative |
| Python class/function metadata view and pickle/reflection observations | Rust transaction + native CPython AST/runtime | Python determines compatible descriptors and serialization |
| Cost policy | Rust | Deterministic heuristic using measured native workloads; not an optimality proof |

The compiler toolchain is **Rust 1.99.1**. No third-party Rust crates are required. The implementation links to system `libsqlite3` and `libcrypto` and invokes Git; the published source package does not redistribute the Rust compiler.

## What the differential checks establish

### v0.10 retained parity

The six relational tables `tree_entry`, `module`, `dependency`, `symbol`, `api_binding`, and `hazard` match Python v1's **full canonical row fingerprints** across **16 revisions** of h11, Click, Requests and PostCSS: 720 tree-entry rows, 400 module rows, 1,828 dependency rows, 4,576 symbol rows, 548 API-binding rows and 12 hazard rows. These are local Git snapshots of published source packages, not full upstream histories.

The two-branch h11 integration reproduces Python v9's exact `guard_sha256`: `1c9aeb621962526acc4c49c69e95da123b89cab77b8ab739244416d6913016da`, including the 3 + 13 carried observations, a clean Git merge, two new joint checks, and the independent 16-test audit.

### Newly ported operations

| Reference | Differential acceptance | Result |
| --- | --- | --- |
| v2 source transactions | h11: 6 import rewrites, PostCSS: 7; exact output Git commit/tree | Pass |
| v3 workload decision | Recompute recorded h11/PostCSS costs and choices | Both choose original |
| v4 historical mapped replay | Historical two-file h11 and PostCSS changes; exact projected Git commits/trees | Pass |
| v5 contract preservation | Original pinned Python checker rerun over **112** traces; three distinct statuses and five contract ideals | Pass |
| v6 source-impact | Whole-package 13-input h11 checks before and after history replay | 11/13, 13/13, 8/13, 8/13, no unsupported promotion |
| v8 Python identity | Original and historical h11 identity projections, 12 entities each, exact Git commits/trees, reflection and pickle protocols 4/5 | Pass |

Exact original Git commits include h11 v2 `601de6db5e094e22c68fb74a976f5beb75c5a4a0`, PostCSS v2 `be0339c42abac5846049550b8290dbe6bf7b4849`, h11 mapped historical replay `9bc063630c5447f28d08b519fc17cd2e943f61ae`, PostCSS mapped replay `ca5be16826310fcd7d966f5d75f3852396d52b8f`, and h11 identity-after-replay `14a4b08e89b1aa12786ab56da9c1d850a9b4fde0`.

The historical h11 change is exact changed-file upstream Git-blob evidence; the PostCSS experiment is an **authentic historical patch transplanted onto an earlier published source snapshot**. It is not an exact original upstream final revision.

## Reproduce from a clean extraction

Requirements: Linux x86-64 with Rust 1.99.1, Git (SHA-1 object repositories), SQLite and libcrypto development libraries, CPython 3.13, pytest, Node.js, TypeScript available to Node's module resolver, and standard shell tools. Check environment before comparing timings. The Rust 1.99.1 compiler can be restored from previously pinned offline toolchain artifacts. The bundled source and Git fixtures do not require remote downloads. No output should overwrite a source checkout.

```bash
cargo fmt -- --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
cargo build --offline

# The retained v0.10 16-snapshot and two-contributor baseline:
bash scripts/reproduce.sh /tmp/relgit-v10-check

# New transactions, history, contracts, identity, impact, and cost:
python3 scripts/reproduce_v11.py /tmp/relgit-v11-check
```

Both reproduction directories must be **empty or nonexistent** before execution. `scripts/reproduce_v11.py` uses **only its bundled Git repositories, fixtures, pinned native probes, and Rust CLI**, and writes its verification report to `<work>/verification.json`. Python scripts under `scripts/` are test orchestration; none implements the Git/SQLite transaction logic. The latest independently produced report is in [`results/verification.json`](results/verification.json).

### Command interfaces

```text
relgit index DB NAME SOURCE [REF] [ROOTS_CSV]
relgit tree-check DB NAME COMMIT
relgit carry EXPERIMENT_ROOT [DB] [REFERENCE_REPORT]
relgit project SPEC_JSON DATABASE OUTPUT_DIRECTORY
relgit replay SPEC_JSON DATABASE
relgit contracts-import SPEC_JSON DATABASE
relgit contract-impact SPEC_JSON DATABASE
relgit identity SPEC_JSON DATABASE
relgit score COST_INPUT_JSON
relgit impact DB NAME COMMIT MODULE
relgit review DB NAME OLD NEW
relgit dependencies DB NAME COMMIT
relgit manifest DB NAME COMMIT
relgit json-check JSON_FILE
```

See `scripts/projection_parity.py`, `historical_parity.py`, `contracts_parity.py`, and `reproduce_v11.py` for complete pinned example spec JSON. Each action refuses unsupported inputs with a nonzero exit code. The Rust optimizer's input is a deliberate scored workload with verified flags and explicit weights, not a request to discover an ideal layout automatically.

## Rust tests and rejection cases

- **17 Rust unit tests** including strict JSON, SHA-1 path/hash/foreign-key integrity, historical patch-scope refusals, workload decisions, and a symlinked-output-path rejection.
- **3 Rust adversarial integration tests** for pinned h11 bounded evidence and corrupted inputs.
- The standalone v11 reproducer additionally requires five refusal cases: path traversal, destination collision, stale source revision, undeclared logical module, and forged candidate observation trace. It also checks Git fsck and SQLite foreign keys. See `results/verification.json`.
- `cargo fmt -- --check`, `cargo clippy --offline --all-targets -- -D warnings`, and inherited v10 reproduction pass.

## What is *not* established

- **Not a full port of every old experiment.** The broader synthetic explanatory-history constructors, exploratory discovery/ranking scripts, and large-scale workload-generating benchmarks remain research tooling until their contracts justify a stable Rust API. We did not replace them with unsupported Rust versions.
- Native language syntax and runtime behavior still belong to Python and TypeScript adapters. Their AST/TS compiler outputs are checked by Rust, but no parser is infallible and dynamic imports/reflection may remain unsupported.
- Matching snapshots/finite probes do **not** prove universal Python or JavaScript observational equivalence. Python physical paths and `__spec__` still differ after relocation.
- Contract evidence is unsigned, finite, and conditional on its declared runtime assumptions. The source-identity decision `SOURCE_MATCH_ONLY` is not fresh test execution; relocated layouts remain `UNVERIFIED_PROJECTION` in v6-style source-identity checks even when newly tested finite observations pass.
- Git writes and SQLite commits are not one atomic cross-storage transaction. An interrupted job can leave an orphaned generated Git checkout requiring cleanup. The engine does not claim crash-safe distributed transactions.
- Subprocess adapters can hang or fail; the **test reproducer** applies timeouts, but Rust's general-purpose adapter operations are not yet comprehensively sandboxed or time-bounded. Do not execute untrusted code without OS isolation.
- The workload policy is a manually weighted heuristic tested on tiny deterministic workloads, not measured developer throughput or a proven productivity advantage.
- The current Git OID implementation requires SHA-1-format repositories. It fails closed for SHA-256 Git repositories.
- No remote GitHub repository or branch was changed.

The architectural test remains economic as well as semantic: keep this machinery only where it demonstrably improves integration or verification beyond ordinary Git and conventional CI.
