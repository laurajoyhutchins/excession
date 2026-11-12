# Exact Git source changes

This PR adds \`excession changes . --base REV [--head REV]\`. It reads two immutable Git commits and compares tracked blob paths, modes, and OIDs without touching the worktree. It intentionally excludes trees and gitlinks from its **blob** inventory. It does not claim semantic change impact.

Changes are represented as ADDED, DELETED, MODIFIED, or MODE_CHANGED, with both before/after source references and null for a nonexistent side. Identical content moved to a new path is reported as delete + add rather than an unearned verified rename.

The output uses the scaffold \`excession.change-inventory.v1\` JSON Schema. Report/change IDs are reproducible digests, **not** signed attestations or proof of independent correctness.

The test module checks exact classification and mode handling. Real two-commit Git fixture acceptance remains a release prerequisite. No ontology concept claims are used to infer source changes; #94 annotations remain optional.
