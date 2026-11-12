# Implementation: replayable evidence explanations

`excession explain . --report saved-impact.json --id CLAIM_ID [--json]` recomputes the entire source-bound impact report from pinned commits and the current native parsers. It rejects changed or invented reports rather than treating a saved JSON document as authority. IDs are deterministic SHA-256 digests of the claim (excluding its ID field).

`excession explain . --report saved-impact.json --id CLAIM_ID --audit` emits a separate audit result: Git authentication and replay PASS, structural schema validation NOT_RUN, independent semantic correctness NOT_EVALUATED. This is not a proof of safety, exploitability, or semantic correctness. A future independent oracle is required to change the last status.

Impact paths remain STATIC_IMPORT candidates and are bounded to the original path evidence. Concept annotations proposed in #94 may eventually appear as optional context; they must not upgrade candidate proof or fabricate dependency edges.

The saved impact JSON is an *input receipt*, not a new database. There is no new source or graph authority.
