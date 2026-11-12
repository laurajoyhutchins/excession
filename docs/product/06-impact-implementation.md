# Implementation: bounded Git-grounded impact

Run \`excession impact . --base REV [--head REV] [--json]\` from a supported repository. This performs exact Git blob change enumeration and source-anchored static analysis at **both** commits. Deleted inputs are considered under the base-revision relationships, never invented as surviving head objects. The output \`excession.impact.v1\` includes the change inventory receipt, candidate reverse paths with original source/evidence references, and coverage/unsupported gaps.

Current supported path kind is **STATIC_IMPORT candidate**, not a complete cross-language or build/runtime impact claim. Reuse \`analyze::run\` (itself using the existing SQLite index and \`views::dependencies\`) rather than creating another stored graph. The closure is bounded to 8 hops and 512 effects; truncation is explicitly reported. All impact assessments from static candidates remain \`CANDIDATE_PATH\`.

A missing path is unknown, not a safe-to-skip signal. The \`safe_to_skip_unlisted_tests\` flag is always **false**. Uncovered language changes are reported. Optional future ontology #94 links can annotate concept context but cannot supply synthetic graph hops or behavioral certainty.

TDD: active schema rejection and Rust reverse-edge semantic unit tests; a real two-commit Git fixture checks importer candidates, source revisions and no test-skipping. Deeper independent-oracle acceptance remains tracked by #112.
