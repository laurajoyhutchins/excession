# Implementation: read-only repository discovery

Run \`excession discover .\` (optionally \`--rev COMMIT\`) to inspect a pinned Git revision. This does not execute source code, write a database, or modify the source checkout.

The report is \`excession.discovery.v1\`: exact commit/tree, source-root directories, capability observations from \`doctor\`, proposed external cache location, and explicit omissions.

The current module indexer does not handle tracked source files at repository root, so those are excluded and disclosed. Only Python and JavaScript/TypeScript source roots are selected for the current static indexer. C/C++/Rust build semantics require configured native tools and are not inferred by this preflight.

Dirty working trees are permitted for this **read-only Git object** preflight. This is not permission to execute analyzers or scanner code from a dirty tree. An \`AVAILABLE\` tool is a detected capability, not proof that a repo's build configuration is supported.

Auto-ontology PRs #88–#95 are independent: discovery does not compile concept models or infer source dependencies from concept similarity.
