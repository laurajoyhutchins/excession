# Implementation: one-command static candidate analysis

\`excession analyze .\` indexes discovered Python and JavaScript/TypeScript source directories using Excession's existing source-authenticated Rust/SQLite indexer and upstream native language parsers. \`--json\` emits \`excession.analysis.v1\`; otherwise human-readable summary text is printed.

The existing \`excession analyze clang\` command remains supported.

The analysis report contains exact Git source identities, source-linked **static import candidates**, deterministic observation fingerprints and coverage gaps. A fingerprint is an unkeyed identifier for an indexed observation, not independently attested native execution. Report coverage is always PARTIAL because dynamic code, external imports, configuration differences and non-indexed languages remain outside this implementation.

All SQLite data is written to a predictable cache outside the source repository. \`--db\` must name an absolute destination outside the source tree. If a required parser is unavailable, analysis fails with an explicit diagnostic rather than silently omitting that language.

Auto-ontology #88–#95 remains independent. There is no implicit concept compilation or upgrade of a static import candidate to a semantic guarantee.

Real Git integration and independent import oracle acceptance remain required before this is considered production-ready.
