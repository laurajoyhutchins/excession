# Git reconstruction evidence replay

`excession reconstruct git-evidence --repo <checkout> --plan <plan.json> --db <evidence.sqlite> --evidence <bindings.json>` extends the read-only structural verifier with *fresh replay* of existing build-provenance and REA artifact evidence. It neither builds nor executes reconstructed code. No new database or receipt producer is introduced.

A binding references a genuine checkpoint and an introduced contract (or an explicitly declared integration node and retained contract). Each binding requires **both** the `build_snapshot` and a linked `rea_bundle` already stored in Excession's shared evidence SQLite database:

```json
{
  "schema":"excession.git-reconstruction-evidence.v1",
  "bindings":[
    {
      "phase":"INTRODUCTION",
      "checkpoint":"<exact Git SHA-1 commit>",
      "contract":"read",
      "artifact":"/absolute/path/to/preexisting/build/artifact",
      "build_snapshot":"<previously recorded SHA-256 evidence snapshot>",
      "rea_bundle":"<previously imported SHA-256 REA bundle ID>"
    }
  ]
}
```

Receipt creation is handled by the existing `excession evidence build-record` and `excession evidence rea-import` commands. `git-evidence` reruns their independent audit code, pins their source revision to the **checkpoint**, compares the exact artifact SHA-256 across both records, and verifies that every path in the declared source closure occurs among the authenticated build input declarations. A wrong revision, modified source blob, missing build input, changed artifact bytes, absent or unlinked REA bundle, duplicate binding, or unsupported field is rejected.

Missing evidence stays visible in `missing_bindings`, without falsely accepting an unverified contract. `receipt_coverage=COMPLETE_REPLAYED_BYTES_ONLY` means all expected bindings replayed. It **does not** mean a build actually executed using the claimed inputs, an REA semantic predicate is correct, or a contract survived integration. Those remain `NOT_ESTABLISHED`/`false` even when no binding is missing. REA here authenticates an imported Evidence v2 bundle and exact artifact digest, not an independent behavioral oracle.

This is the source/artifact provenance seam for future isolated contract-oracle execution. A future oracle executor must provide independently checked evidence and joint integration checks, without collapsing structural coverage into proof of behavior. Git tree identity remains the target; the earlier binary `.text` reconstruction stays separate.

Current limitations: Git SHA-1 only, explicit source dependency and contract closures, local artifact files required, read-only already-recorded receipts, no automatic build execution, no trusted test-oracle replay, no independent build-process attestation.
