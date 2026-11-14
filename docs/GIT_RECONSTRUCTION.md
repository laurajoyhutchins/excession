# Git reconstruction verifier

`excession reconstruct git --repo <checkout> --plan <plan.json>` replays an
explicit candidate construction plan **without writing to Git or executing
source code**. The original Git tree remains the authority for every blob and
mode. This complements, but does not replace, `reconstruct evaluate`'s
trusted-local compiler comparison.

A plan contains a pinned `target_commit`, explicitly declared
`source_dependencies`, source-bound `contracts`, and one or more named
`histories`, and an `integration_nodes` array (which may be empty). Each history contains pinned Git commit OIDs with lists of contract
identities introduced at those checkpoints. An example for a simple system:

```json
{
  "schema":"excession.git-reconstruction-plan.v1",
  "target_commit":"<pinned 40-character SHA-1 commit>",
  "source_dependencies":[
    {"path":"reader.rs","requires":["common.rs"]}
  ],
  "contracts":[
    {"id":"read","source_closure":["common.rs","reader.rs"],"requires":[]}
  ],
  "histories":[
    {"name":"reader-first","checkpoints":[
      {"commit":"<pinned foundation commit>","introduces":[]},
      {"commit":"<pinned completed commit>","introduces":["read"]}
    ]}
  ],
  "integration_nodes": []
}
```

The verifier checks:

- Real commit OIDs and ancestry, with no need to rewrite authentic history.
- Every checkpoint's paths, modes, and blob OIDs against the target.
- Monotonic source-path inclusion along a declared history.
- Every *declared* source dependency at every checkpoint.
- Complete contract source closures and prerequisite contracts at introduction.
- Retention of introduced source closures at all subsequent stages.
- Exact final Git tree identity for **every** history, even if commit IDs differ.
- Optionally, actual two-parent Git merges, their exact parent identities, parent source union,
  and preservation of separately complete declared contract closures. Every integration
  must be reachable from at least one declared final history.

It rejects extra paths, altered source blobs, missing dependencies, premature
contract introductions, missing completion, and non-descendant checkpoints.
Gitlink repositories are currently refused. Object format is SHA-1, matching
Excession's existing Git adapter. The input is bounded to 1 MiB, eight
histories, 64 checkpoints per history, and 64 explicit integration nodes.

## Evidence boundary

`EXACT_TARGET_TREE_AND_DECLARED_CLOSURES` means a proposed history is a
structurally valid reconstruction **relative to the declared dependencies**.
No static dependency list is assumed complete. Commit parentage cannot establish
semantic prerequisites, nor is a synthetic history a record of authentic
chronology. A valid snapshot is not proof that an intermediate commit is
installable, functionally correct, or safe to merge.

Build provenance, REA evidence, independent contract oracles and integration
behavior are always explicit `NOT_ESTABLISHED` obligations in the report. The
verifier cannot promote caller-written receipts to verified evidence. Source
identity preservation is not behavioral preservation. The next integration
should bind existing verified receipts to the report and replay independent
oracles at capability introduction and joint integration, not duplicate the
build/REA stores or invent a second authority.

To iterate, revise the candidate plan or produce new Git commits, then invoke
the verifier again. It is an exact-object checker, not an automatic history
synthesizer.


Executable, bounded source-bound oracle replay is specified in [GIT_RECONSTRUCTION_ORACLES.md](GIT_RECONSTRUCTION_ORACLES.md). The execution path explicitly requires `--trusted-local true`; it does not sandbox target code or establish universal behavioral equivalence.
