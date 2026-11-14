# Deterministic Git reconstruction synthesis

`excession reconstruct git-synthesize` builds **candidate**, source-identical Git histories from one pinned tree and explicitly declared dependency and contract edges. It does not invent original chronology or infer semantic completeness.

```sh
excession reconstruct git-synthesize \
  --repo /absolute/path/to/checked-out-source \
  --seed /absolute/path/to/seed.json \
  --output /absolute/path/to/new-directory \
  --max-candidates 2
```

The command creates `output/repository` (a separate local Git repository), `output/plan.json` (the existing `excession.git-reconstruction-plan.v1` format), and `output/report.json`. The source checkout is never modified. The new output directory must not exist and cannot be inside the source checkout.

`seed.json` uses `excession.git-reconstruction-seed.v1`, consisting of an exact `target_commit` and declared `source_dependencies` and `contracts` with their `source_closure` and contract prerequisites. Only **actual source Git blobs and modes** are used. The generator checks contracts for dependency closure and cycles, creates independent root commits using Git plumbing, and calls the existing structural verifier on the resulting real Git histories. The final tree is required to be identical to the pinned target. It never evaluates source code or runs untrusted hooks.

For **two explicitly independent contracts**, synthesis creates parallel capability commits and a real two-parent Git merge, with both parent orders represented as valid explanatory histories. It automatically selects one joint integration obligation and introduction obligations. For other contract sets, it emits up to eight distinct deterministic prerequisite-compatible serial orders and their introduction obligations. It does **not** pretend that a serial order establishes integration safety. The planner has a bounded heuristic, not an optimizer claiming a unique best history.

Each `oracle_selections` item records the exact synthetic checkpoint, Git tree, contract or joint integration set, and `NEEDS_INDEPENDENT_ORACLE`. These are requests, not executed oracles: a separate trusted check and build/REA receipts must be supplied to `reconstruct git-evidence` and `reconstruct git-oracles`. This keeps all three evidence layers required and avoids running untrusted agent outputs without isolation.

To incorporate a counterexample from an independent oracle, pass `--feedback /path/to/failures.json`. The `excession.git-reconstruction-counterexamples.v1` format contains an array of `{ "contract": "...", "tree": "40-char Git tree OID", "oracle_sha256": "64-char SHA-256" }`. **This manifest is caller-supplied, not authenticated evidence.** A matching state is flagged `COUNTEREXAMPLE_REQUIRES_REPAIR`. If an oracle rejects the final target tree, all histories preserving that exact tree are blocked as behavioral candidates. Reordering identical source bytes never repairs a substantive implementation error. Preserve the original failed execution evidence in Excession's evidence store when available; do not elevate feedback to a proof.

Limitations: declaration completeness is unknown; the generated early histories may require temporary test-only import shims; generated scratch commits represent possible orders, not original development events; Git SHA-1 is a legacy object identifier; semantic correctness, isolation, reproducible build execution, and agent productivity are **NOT_ESTABLISHED**.
