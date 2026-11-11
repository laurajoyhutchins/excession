# Refactoring hypotheses (bounded v1)

Excession reports structural **candidates for review**, not automatically safe
edits or verified improvements. The first slice deliberately reuses its indexed
Python/JavaScript module graph and symbol fingerprints.

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

## Planned reuse, not implemented in v1

SCIP is the natural resolved-symbol index format. jscpd can report clone
families beyond exact fingerprints; ast-grep can match patterns; Code Maat's
methods can inform historical co-change; and language-native refactoring tools
should perform any eventual edits. All imported results would require explicit
source binding, tool/version provenance, and independent validation.

`tests/test_refactor_cli.py` creates a local Git fixture and verifies
cycle/duplicate detection, Git identities, deterministic replay, report and
SQL-relation tampering, and unindexed-revision rejection.

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
