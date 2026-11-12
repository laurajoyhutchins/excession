# Installed CLI end-to-end acceptance

The extracted release archive is tested from a separate Git repository through static source analysis, exact revision impact, evidence explanation, and replay audit. The fixture changes an imported JavaScript module, checks that the consumer remains a candidate, then mutates the saved impact JSON and demands refusal.

This extends the existing portable workflow without a second packaging pipeline. Ordinary PRs keep Linux smoke; release/manual jobs exercise the complete portability matrix. A successful run establishes the bounded fixture behavior, not semantic completeness or vulnerability validity.

Auto-ontology #88–#95 remains optional and cannot supply dependency observations.
