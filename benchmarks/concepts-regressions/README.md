# Existing-dataset regression oracle pilot

## Source dataset and provenance

This pilot uses the actual RegMiner (ESEC/FSE 2022) artifact-evaluation document,
pinned at dataset repository SongXueZhi/RegMiner commit
`de00159618041bd1be936cf8d4ab07077ff58f61` and document Git blob
`8436ab781160a1976def99ac0c87cc7def80b80c`. The three reported
univocity-parsers regression cases (Ticket_13, Github_309, Github_24)
are in `regminer-2022-demo.v1.json`, with full Git SHA-1 commits and exact
TestNG method identifiers. Original source credit remains with RegMiner.

**RegMiner migrates tests across revisions.** In all three cases the
test added by the repair C does not exist at the working A or inducing B.
Before the evaluator fetches C, Excession freezes a prediction from the
existing temporal model using source commits A and B only. After freezing,
a test runner copies the exact Git blob of C's new test to checkouts of
A and B. The runner does not change production source code.

The source projects use old Java 6 target settings and TestNG. CI runs on
an isolated ephemeral Ubuntu runner with Java 8 and Maven; artifacts
include Maven logs and Surefire results. A Maven build failure, dependency
failure, missing executed testcase, or timeout yields **UNSUPPORTED**.
Only an executed matching testcase can return PASS or FAIL.

## Native Rust adjudicator

Run after obtaining a frozen prospective report and externally produced
three-phase oracle receipts:

```sh
excession concepts regression-adjudicate \
  --repo /isolated/univocity \
  --dataset-repo /isolated/RegMiner \
  --manifest benchmarks/concepts-regressions/regminer-2022-demo.v1.json \
  --case ticket-13 \
  --frozen frozen.json --oracle oracle.json
```

The command is read-only. It checks the exact documentation Git blob,
the source commits, A as B's first parent, B as C's ancestor, the
migrated test's C blob, the frozen ontology source tree and candidate
revision, and A/B/C oracle source trees and phase ordering. It refuses
missing or tampered identities, and preserves explicit unsupported status.

**Claim boundary:** Caller-supplied oracle receipts are not independently
authenticated by Rust. The CI runner logs bind them to observed commands,
but that is not cryptographic producer attestation. A successful A/B/C
replay establishes a behavioral observation, not an automatically
discovered architectural invariant. Because the test did not exist at A,
a source-file predictor at A cannot be expected to name that future test.
File-prediction recall on that test is marked not comparable; do not
claim a superiority result over static or mature test selectors.

## Next dataset adapter

BugsInPy supplies real buggy/fixed Python pairs and executable test scripts
but generally not a verified preceding working commit A. These are
two-phase controls until an A revision is independently established.
Do not manufacture A from the parent of B or call B/C evidence A/B/C.

CI run and artifacts: see `.github/workflows/concepts-regminer.yml`.
