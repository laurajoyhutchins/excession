# PR 5: Exact Git change inventory

**Status:** documentation, schema and TDD only. No new diff implementation.

Define a source-bound `excession.change-inventory.v1` for two exact Git commit/tree identities. Preserve old and new blob/mode/path and represent absent sides as null. Existing `git::changed`, `index::changed_paths` and `views::review` provide a starting point, but a --name-only Git diff does not establish content/mode detail.

Rename detection is **candidate-only**, including identical-blob moves. Deletions need base-revision evidence. Git source facts never derive from semantic concepts; auto-ontology #88–#95 may annotate changes later through #94 without altering source identities.

**Tests:** positive/negative wire tests now; independent Git oracle pending for add/delete/mode/rename, pinned source mismatches, dirty worktree parity, and non-UTF-8 path refusal.

Tracks #80, #85 and foundation #101.
