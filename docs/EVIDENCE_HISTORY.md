# Historical evidence transport

**Status: implemented, source-closure scope only.** Excession treats Git
commits as immutable computational states. An agent, human author, merge
strategy, or commit timestamp is not part of the transport semantics.
Any two retained revisions in the same SHA-1 Git object database may be
compared, including divergent branches and synthetic merges.

## Model

An observation originates at a Git commit and a declared dependency
closure. A relation to another commit records which inputs changed, not a
new passing test or a proof of semantic equivalence.

```text
Git commit C0 -- source closure S0 -- finite observation R0
       |                                  |
       +---- target C1, closure S1 <-----+ source comparison
       +---- target C2, closure S2 <-----+ source comparison
       +---- target M,  closure SM <-----+ source comparison
```

All edges point back to the same historical receipt. No agent identity,
ancestry relation, ref name, or checkout mutation is required. An edge
with `SOURCE_MATCH_ONLY` means only that the **declared** Git source
dependencies, entrypoint and declared assets are unchanged. An edge with
`REVERIFY_REQUIRED` reports changed paths or entry/asset assumptions.
Neither state asserts passing behavior on the target.

## Reproduce on a real Git history

Provide a reviewed Python test entrypoint and explicit non-Python assets.
The default `plan` command retains its strict clean-HEAD behavior; add
`--history` to reconstruct any retained revision exclusively from Git
objects. There is no checkout or worktree switching.

```bash
REPO=/path/to/git/checkout
SOURCE=$(git -C "$REPO" rev-parse OLD_COMMIT)
TARGET=$(git -C "$REPO" rev-parse NEW_COMMIT)
DB=/tmp/excession-history.sqlite

python3 tools/excession_slice.py plan "$REPO" tests/check.py \
  --revision "$SOURCE" --history > /tmp/source.json
python3 tools/excession_slice.py materialize \
  "$REPO" /tmp/source.json /tmp/old-slice
# Only execute reviewed source on a machine with the required isolation:
python3 tools/excession_slice.py run \
  "$REPO" /tmp/source.json /tmp/old-slice \
  --trusted-local --isolated-linux > /tmp/source-receipt.json

excession slice ingest --repo "$REPO" --manifest /tmp/source.json \
  --receipt /tmp/source-receipt.json --db "$DB"
excession slice audit --repo "$REPO" --manifest /tmp/source.json \
  --receipt /tmp/source-receipt.json --db "$DB"

python3 tools/excession_slice.py plan "$REPO" tests/check.py \
  --revision "$TARGET" --history > /tmp/target.json

excession evidence transport --repo "$REPO" \
  --source-manifest /tmp/source.json --target-manifest /tmp/target.json \
  --receipt /tmp/source-receipt.json --db "$DB"
excession evidence transport-audit --repo "$REPO" \
  --source-manifest /tmp/source.json --target-manifest /tmp/target.json \
  --receipt /tmp/source-receipt.json --db "$DB"
excession evidence trace --repo "$REPO" \
  --receipt-id RECEIPT_SHA256 --db "$DB"
```

The output `transport_id` identifies a canonical, source-bound comparison.
It does **not** represent a relocated successful test result. `trace`
reads the stored origin receipt and transport edges, independently rebuilds
their source bindings from Git, verifies the SQLite rows and refuses
inconsistency. Only direct relationships to a given receipt are displayed,
with a 256-edge limit. It does not infer transitive proof preservation.

## Evidence and trust boundaries

- Rust validates commit/tree identity and all declared source paths, Git
  blob modes and bytes, SHA-256 digests, and exact stored SQL rows.
- Python rederives an import closure from Git source, but Rust cannot prove
  that Python's static analysis includes every runtime dependency.
- A historical commit alone **does not supply a historical test pass**.
  Only a previously obtained finite observation may become an origin.
- Execution receipts remain **unsigned local claims**; a hash does not prove
  that execution happened on a trusted remote worker.
- External state, environment variables, dynamic imports and nondeterminism
  can alter behavior even when the declared static closure is identical.
- Thus `SOURCE_MATCH_ONLY` is not permission to skip integration tests.
  Stronger property preservation will require explicit/enforced effect
  contracts, or independently established semantics for each claim.

## Verification

Run `make check` on Rust 1.99.1. The historical integration tests construct
small real Git histories with two divergent branches and a merge, transport an
observation from an earlier commit while HEAD is elsewhere, detect a changed
dependency causing a failing target test, and reject a forged target manifest,
a missing origin receipt, and mutated SQLite evidence.

No GitHub Actions or remote worker attestation is required for these local
source-identity claims. The history graph is intentionally useful without
introducing agent-specific orchestration.
