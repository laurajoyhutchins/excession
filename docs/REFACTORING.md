# Refactoring intelligence

Excession finds **places to investigate** in an indexed codebase and ties
each suggestion to the Git revision and source observations that produced it.
The built-in detector uses Python/JavaScript module relations and body
fingerprints; a separate SCIP path consumes language-native symbol references.

These are evidence-backed hypotheses, not automatic edits or proven
improvements. Use the language's own refactoring tools and tests to evaluate
any proposed change.

## Usage

```sh
excession index --repo /path/to/repo --db /tmp/index.sqlite \
  --name project --rev HEAD --roots src,tests
COMMIT=$(git -C /path/to/repo rev-parse HEAD)
excession refactor suggest --db /tmp/index.sqlite \
  --name project --rev "$COMMIT" > /tmp/refactor.json
excession refactor audit --db /tmp/index.sqlite \
  --name project --rev "$COMMIT" --report /tmp/refactor.json
```

Output schema: `excession.refactor-candidates.v1`. Each report contains
the exact Git commit/tree, indexed coverage, a SHA-256 report ID, and a sorted
set of source-bound candidate records. Each candidate has a content-derived ID
and investigation/verification instructions. No source files are modified.

## Candidate classes

- `dependency_cycle`: strongly connected groups of two or more indexed
  modules, with source paths, blob IDs and internal dependency observations.
  A cycle is not automatically a defect.
- `duplicate_body_fingerprint`: functions in different modules with
  identical language-specific parser fingerprints, kinds, and signatures.
  Python AST and JavaScript normalized-source fingerprints are not comparable.
  A matching fingerprint does not establish semantic equivalence.

These findings are **not ranked remediations**. A proposed consolidation still
needs contract, caller, side-effect and test review.

## From hypothesis to change

A useful refactoring report tells you **what was observed**, **where**, and
**why that observation may matter**. For each candidate, inspect the exact
source paths and Git blobs, check callers and tests, then perform a change
using the appropriate language-native tooling. Re-run the index and audits
after changing source; the old report remains evidence about its original
revision, not approval of the new one.

## Authority and limits

The Git revision and root tree are checked against independently reconstructed
SQL tree entries. `audit` reruns the query and rejects altered reports or
changed indexed relationships. It does **not** independently rerun the Python
or JavaScript parsers or prove that indexed relationships were correct.

Dynamic imports, runtime dependencies, other languages, unindexed directories,
and external packages are outside coverage. Missing edges are unknown, not
absent. Output never certifies an executable transformation or authorizes
skipping tests.

Analysis refuses excessive inputs: 10,000 modules, 50,000 indexed edges,
100,000 symbols, 128 candidates, or an 8 MiB report. Narrow index roots to
reduce scope.

## Possible extensions

SCIP ingestion and source-bound symbol-reference hypotheses are already
available through the separate commands documented below. Further reuse
could include jscpd for clone families, ast-grep for structural patterns,
and Code Maat's methods for historical co-change. These integrations are not
implemented here. Source binding and producer-specific semantics would still
be required.

`core/tests/refactor_contracts.rs` creates a local Git fixture and verifies
cycle/duplicate detection, Git identities, deterministic replay, report and
SQL-relation tampering, and unindexed-revision rejection.

## SCIP fixture migration

Pinned real and historical repositories are evaluated using upstream SCIP
indexers and the existing source-bound SCIP replay. The acceptance contract is
**Git binding, replay, and explicit scope accounting**, not agreement with the
former Python/JavaScript import resolver. The old import oracle continues to
test a separate contract, and SCIP symbol references are not promoted to import
edges. See [SCIP fixture migration](../benchmarks/experiments/SCIP_MIGRATION.md).

## Optional SCIP symbol evidence

SCIP is Excession's preferred interchange format for definitions and references
reported by **language-native indexers**. Install the official `scip` CLI and
an appropriate indexer separately. The analyzers may compile or inspect a
project, so run unknown builds only in isolated workers.

```sh
# Generate index.scip using the appropriate language-native indexer.
scip print --json index.scip > /tmp/index.scip.json
excession refactor scip-suggest --repo . --rev HEAD \
  --index /tmp/index.scip.json --source-prefix core > /tmp/scip-refactor.json
excession refactor scip-audit --repo . --rev HEAD \
  --index /tmp/index.scip.json --source-prefix core --report /tmp/scip-refactor.json
```

This is **official SCIP-decoded evidence input**, not a new Protobuf decoder.
For a Rust crate in `core/`, run `rust-analyzer scip ./core --output /tmp/index.scip`,
then `scip print --json /tmp/index.scip > /tmp/index.scip.json`. The explicit
`--source-prefix core` checks the upstream SCIP project root against the
pinned Git checkout without rewriting its document paths. Indexing is
still a separate, trusted-local preparatory step, not a sandboxed operation. We reuse the upstream
SCIP decoder and inspect its Go JSON field names (e.g. `relative_path`,
`symbol_roles`, and `TypedRange`); newer typed ranges and legacy `range`
arrays are supported. No new Cargo dependencies are needed.

Every document path must match a tracked regular file in the pinned Git
revision. Blob bytes and source line/character spans are checked against
Git objects. The report records the raw decoded index SHA-256, indexer
name/version, explicit languages, matched and unresolved reference counts,
and a content-addressed report ID. Audit independently recreates the source
bindings and report, but does **not** independently rerun the indexer or
establish its semantic correctness/freshness.

The first two SCIP-specific hypotheses are:

- `reciprocal_symbol_references`: two source documents explicitly refer to
  symbols uniquely defined in each other. This is *not* a verified import or
  compile dependency cycle.
- `cross_language_symbol_boundary`: one source document explicitly refers
  to a uniquely defined SCIP symbol in a file with another language.
  Neither matching names nor separate independent index outputs establish
  a cross-language relationship.

Indexers have different coverage and may omit references, incomplete build
configurations, external definitions, FFI, generated code, or dynamic calls.
A SCIP reference is not automatically a call, import, or safe change target.
Input bounds: 16 MiB JSON, 1500 documents, 100,000 occurrences, 512 KiB per
source blob, 32 MiB total source, and 128 candidates. Missing/unmatched symbols
are reported as unresolved rather than guessed.

The next extension is to add multi-index manifests, which can combine symbol
evidence from independently version-pinned indexers while preserving each
indexer's own provenance and configuration.

## Advisory SCIP refactoring triage

The SCIP report keeps every candidate, original Git/blob/source reference,
and the original content-addressed `candidate_id`. A separate `triage`
section partitions those IDs into `review_candidate_ids` and
`likely_structural_module_candidate_ids`. Review candidates appear first
in the `candidates` array. The audit regenerates and checks the entire
report, including triage, from the same Git revision and index bytes.

The narrowly scoped structural heuristic requires a `rust-analyzer` index
of Rust documents, a `src/lib.rs` or `src/main.rs` crate root and a
same-directory `*.rs` sibling. The **only** indexed root-to-sibling reference
must be that sibling's exact module symbol (for example `semantic/`).
An additional native reference to a function or type leaves the pair in
`review_candidate_ids`. Nested parent/child modules are not automatically
demoted. Indexer identity, document paths and source identities are still
checked as before.

This does **not** parse Rust declarations, prove the absence of other
references, establish architectural responsibility or certify a refactor.
It is only a presentation heuristic derived from existing native symbol
observations. Missing, unsupported or differently shaped evidence remains
a review candidate, never a safe-to-ignore finding.

## First deterministic code-generation recipe: `extract-shared-module`

This bounded first recipe operates on **one explicitly chosen source module** from a
source-bound, successfully replayed rust-analyzer SCIP candidate. It does not
invent a source-code parser or infer that a candidate is a defect. The caller
selects 1–8 top-level public `fn` items and a new sibling `src/*.rs` module.

The external [ast-grep](https://ast-grep.github.io/) `outline` command is the
only Rust syntax locator. The recipe passes it **pinned Git blob bytes** over
stdin and validates its exact byte ranges, item kind and names. The recipe
refuses non-top-level items, attributes, adjacent documentation, module-relative
references, duplicate names, unsupported source layouts and pre-existing
destinations. It preserves the original public paths with a `pub use` in the
original source and registers the new module in `src/lib.rs`.

### Preview (no source writes)

```sh
excession refactor extract-shared-module \
  --repo /path/to/checkout --rev <exact-commit> \
  --index /path/to/index.scip.json --report /path/to/scip-report.json \
  --candidate <candidate-id> \
  --source core/src/semantic.rs --destination core/src/new_shared.rs \
  --symbols item_one,item_two --ast-grep /path/to/pinned/ast-grep
```

### Apply (new detached worktree only)

Pass `--output /absolute/new/worktree` to materialize the same candidate
changes in an **absent** directory using `git worktree add --detach` at the
original commit. This never edits the original checkout. Excession does **not**
commit or approve the result, and does not execute hooks, arbitrary scripts,
`rustfmt`, or builds. It reports edited paths, source Git OIDs and generated
content digests.

The result is `UNVERIFIED_CODEGEN_PREVIEW` or `APPLIED_UNVERIFIED`, **not**
a certified refactor. A candidate requires `cargo fmt`, `cargo check`,
`cargo-semver-checks` where relevant, independent tests, and a fresh
rust-analyzer SCIP index. A successful compiler check alone is not proof of
equivalent behavior, and a disappearing reciprocal edge is not proof of safety.

**Scope limitation:** this recipe does not move private structs/types, update
arbitrary imports, infer where functions belong, repair references, or resolve
the richer `Plan`/reuse-selection transformation from PR #161. Those cases
return unsupported until they have a separate mechanically checked recipe.

### Candidate alignment and replayable previews

The executor now refuses extraction when none of the selected functions is the
**target of a native SCIP reference in the chosen candidate**. Additional
explicitly selected helpers may travel with it, and are reported separately
as `additional_helper_functions`. A reciprocal *file* relationship by itself
is not an extraction authorization.

The preview emits a `plan_id` and `edits` array. Each edit is anchored to an
`original_blob` (or null for a new file), exact original `start_byte` /
`end_byte` offsets, and replacement text. Excession independently replays
the edits to every generated output before writing anything. A preview and
subsequent apply over the same pinned source/report return the same
`plan_id`, allowing external tools to compare their candidate patches.

Adjacent `//` and `///` documentation moves with the selected function.
Unsupported attached attributes, module/block comments, and module-relative
references fail closed. Source-root whitespace is preserved byte-for-byte
apart from the new module declaration. Output worktrees inside the original
checkout are refused, and Git checkout hooks are disabled for the
`worktree add` operation. A configured Git checkout filter can still run
during a trusted checkout, so do not treat output worktree creation as a
sandbox for hostile local Git configurations.

The recipe still cannot determine general symbol closures or guarantee that
an unverified source edit compiles. It does **not** automatically execute
untrusted generated code. Compilation and tests remain external receipts.

## Native Rust-aware code actions

The `refactor rust-assist` command uses a real
[rust-analyzer LSP server](https://rust-analyzer.github.io/) rather than
implementing Rust item moves and import resolution in Excession.

- Input: an **exact Git commit**, a clean checkout at that commit, a fully
  audited rust-analyzer SCIP report, a non-structural candidate ID, and one
  source file whose Git blob is in that candidate.
- Selection: an explicit LSP range using **zero-based lines and UTF-16
  characters**.
- Without `--action`, list the server's supported refactoring actions and
  import quick fixes. Discovery is read-only.
- With `--action "<exact title>"`, return a deterministic, Git-blob-bound
  `plan_id`, source-byte `edits` and generated SHA-256 results.
- With an additional `--output /absolute/new/worktree`, materialize the edits
  in a separate detached checkout at that exact revision, never modifying
  the original checkout.

Example (the selected range is intentionally explicit):

```sh
excession refactor rust-assist \
  --repo /clean/source/checkout --rev <exact-commit> \
  --index /tmp/index.scip.json --report /tmp/scip-report.json \
  --candidate <id> --source core/src/semantic.rs --source-prefix core \
  --start-line 15 --start-character 0 --end-line 74 --end-character 0
```

Supported code actions have `refactor*` kinds or native `Import ...` quick
fixes. The provider may return edits to multiple Rust source files, including
new Rust modules. Excession converts the native `WorkspaceEdit` into exact
byte ranges over the original Git blobs and rejects edits outside the
tracked checkout, non-Rust edits, overlapping ranges, rename/delete
operations, stale versions, unsupported URI paths and ambiguous actions.
No compiler/builder is invoked by Excession when applying a patch.

This is a guarded **source transformation**, not an autonomous architecture
optimizer or a soundness proof. rust-analyzer decides what edit to make only
for the explicitly selected native action. All generated changes remain
`APPLIED_UNVERIFIED` until independently compiled, tested, audited for API
breakage and re-indexed. The rust-analyzer executable and the clean checkout
are trusted local toolchain inputs. Proc-macro/build-script evaluation in
rust-analyzer environments must be managed by the calling sandbox policy.
