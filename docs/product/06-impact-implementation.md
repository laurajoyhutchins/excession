# Implementation: bounded Git-grounded impact

Run \`excession impact . --base REV [--head REV] [--json]\` from a supported repository. This performs exact Git blob change enumeration and source-anchored static analysis at **both** commits. Deleted inputs are considered under the base-revision relationships, never invented as surviving head objects. The output \`excession.impact.v1\` includes the change inventory receipt, candidate reverse paths with original source/evidence references, and coverage/unsupported gaps.

Current supported path kind is **STATIC_IMPORT candidate**, not a complete cross-language or build/runtime impact claim. Reuse \`analyze::run\` (itself using the existing SQLite index and \`views::dependencies\`) rather than creating another stored graph. The closure is bounded to 8 hops and 512 effects; truncation is explicitly reported. All impact assessments from static candidates remain \`CANDIDATE_PATH\`.

A missing path is unknown, not a safe-to-skip signal. The \`safe_to_skip_unlisted_tests\` flag is always **false**. Uncovered language changes are reported. Optional future ontology #94 links can annotate concept context but cannot supply synthetic graph hops or behavioral certainty.

TDD: active schema rejection and Rust reverse-edge semantic unit tests; a real two-commit Git fixture checks importer candidates, source revisions and no test-skipping. Deeper independent-oracle acceptance remains tracked by #112.

## Incremental Python indexing

For Python-only source catalogs, the Rust indexer records a completed
`python_index_snapshot` in the existing SQLite database. On a single-parent
commit, it may copy the parent's dependency, symbol, API-binding, and hazard
rows for modules whose Git blob, mode, module name, and path are unchanged.
Changed Python sources are reparsed by the **same CPython AST adapter**, using
the full module catalog for import resolution.

Reuse is conditional on a matching parser/interpreter signature and catalog,
authenticated parent Git tree, and matching completed-row receipt. Corrupted
parent rows are refused. New or removed modules, missing parent receipts,
producer drift, root commits and merges use complete parsing. The completed
snapshot's parsed/reused counts are available for measurement; the existing
public analysis JSON and its source/evidence identities are unchanged.

This is deliberately not used for JavaScript or mixed-language catalogs:
package entrypoints and native resolver options require a separate validity
rule. Reuse is an optimization of **static candidates**, never proof of
behavioral equivalence or permission to skip tests.
