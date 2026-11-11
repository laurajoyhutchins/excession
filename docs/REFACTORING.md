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
