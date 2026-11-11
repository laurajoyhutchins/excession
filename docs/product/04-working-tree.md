# PR 4: Committed Git analysis from dirty checkouts

Status: draft contract and TDD. No production policy change in this PR.

## Behavior
Existing `git::clean` protects workflows currently depending on source checkout purity. Do **not** remove it globally. Separate a read-only path for exact Git-object analysis (which can tolerate unrelated staged/unstaged edits) from scanner/build/runtime paths that require their current trust policy.

Pinned source object reads must give **byte-identical results** for the same commit whether the working tree is clean or dirty. Worktree metadata is informational and has a distinct identity. No staging, index mutation, generated files inside the target repository, or unsolicited execution.

A later explicit working-tree snapshot mode is **out of scope**. Current contract only states `COMMITTED_OBJECTS_ONLY`.

## TDD
Active malformed status and uncommitted-as-committed shape refusals. Pending real Git tests: staged vs unstaged, deleted and untracked files, Unicode paths, clean/dirty parity and unchanged source checkout.

Ontological concept snapshots from #88–#95 can be related only to **pinned Git source**, never silently to unsaved or dirty bytes.

**Refs:** #85, #79, #101.
