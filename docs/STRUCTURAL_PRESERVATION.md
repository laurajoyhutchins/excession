# Structural preservation receipts

**Implemented scope:** an opt-in, source-pinned differential executor for
explicitly reviewed structural candidates. This is an evidence-producing
building block for [transformations](TRANSFORMATIONS.md), not an autonomous
code refactoring engine, a sandbox, a correctness proof, or merge authority.

## What it checks

The caller supplies two full Git commit IDs (base and candidate), plus a
small JSON contract containing the **exact changed Git paths** and explicit
behavioral obligations. Excession independently verifies the commit/tree IDs,
that the base precedes the candidate, and that the actual Git diff paths equal
the declared set. It refuses to run if those checks differ.

For each `differential` obligation it runs the same argv in **two independent
detached worktrees**, requires successful exit on both sides, and compares
the SHA-256 digests of stdout and stderr. A `candidate_pass` obligation
executes only the candidate-side command and requires exit code zero. Both
kinds are mandatory. Command exit codes, actual invocation, source IDs,
bounded output counts, timeouts and hashes are recorded in a versioned receipt.

A successful receipt has status
`OBSERVATIONS_AGREE_WITHIN_EXERCISED_SCOPE`, not `PROVED_EQUIVALENT`.
The report always retains `semantic_equivalence=NOT_PROVED`,
`ready_to_merge=false`, and `safe_to_skip_unlisted_tests=false`.

## Example

Suppose two reviewed Git commits of a small Python project change only
`src/answer.py` while preserving the behavior of the executable entry point.
Put a reviewed contract in `quality/structural-contracts/answer.json`:

```json
{
  "schema": "excession.structural-preservation-contract.v1",
  "changed_paths": ["src/answer.py"],
  "obligations": [
    {
      "id": "entry-point-observation",
      "kind": "differential",
      "cwd": ".",
      "argv": ["python3", "-B", "main.py"],
      "timeout_seconds": 20,
      "max_output_bytes": 1048576
    },
    {
      "id": "existing-regression-tests",
      "kind": "candidate_pass",
      "cwd": ".",
      "argv": ["python3", "-B", "-m", "unittest", "discover", "-s", "tests"],
      "timeout_seconds": 120,
      "max_output_bytes": 1048576
    }
  ]
}
```

Run **only for code that the operator explicitly trusts to execute**:

```sh
python3 tools/ci/structural_preservation.py \
  --repo . --base BASE_40_HEX --candidate CANDIDATE_40_HEX \
  --contract quality/structural-contracts/answer.json \
  --out /tmp/structural-receipt.json --trusted-local true
```

The code also has an owner-only GitHub Actions
`Structural preservation (trusted manual evidence)` workflow. It accepts
immutable Git revisions and an already-committed, reviewed contract under
`quality/structural-contracts/`; it is **never** triggered by arbitrary PR
content. The manual workflow also builds and runs Excession's native `agent verify`
source-impact command and retains its separate `OPEN_OBLIGATIONS`
report alongside the independent execution receipt. This is a different
evidence layer, not a proof that the interpretation is correct. Neither
interface adds execution isolation. Running malicious
test commands can harm the host. The operator is responsible for ensuring
the source, command manifest and execution environment are trusted.

## Independent check boundaries

The *Git source* and changed-path evidence come directly from Git.
The observed process outputs come from actual native execution, not from
Excession's index or proposed dependency graph. This limits a common-mode
indexing error, but tests can still share a mistaken assumption, depend on
environment state, or miss important branches. Matching output is only
evidence for the named inputs and configuration, not all inputs or internal
state equivalence.

The runner records stdout/stderr sizes and refuses to claim complete
observations after timeout or output-budget exceedance. Its temporary files
still require adequate disk resources; it is not a protective resource
sandbox, and it does not attest the host interpreter or tests cryptographically.
It never deletes, commits or merges the original user's source. Temporary
detached worktrees are removed after evaluation.

An incomplete, failing, or unsupported obligation is preserved as
`INCONCLUSIVE` or `FAILED`. In particular, a missing executable cannot
be reinterpreted as a green result.

## How this advances the ratchet

The [cumulative quality ratchet](QUALITY_RATCHET.md) prevents recurrence of
accepted narrow patterns. This structural verifier instead provides
**specific, reproducible acceptance evidence** for a reviewed candidate.
A future automatic edit producer may open a draft PR and request these
checks, but cannot treat the receipts as sufficient merge authority. The
remaining product work is to link source-grounded investigation claims,
precommitted preservation plans, independently attested producer versions,
runtime state effects and full native build/API checks.
