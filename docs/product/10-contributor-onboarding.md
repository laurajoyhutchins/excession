# Maintainer and contributor experience

The root README now starts with \`discover\`, \`analyze\`, \`impact\` and replayable \`explain\` rather than an internal manually configured pipeline. The user must select a real Git base revision and a \`claim_id\` actually emitted by impact, never an invented result. \`--audit\` is explicit about what source replay does and does not verify.

\`CONTRIBUTING.md\` documents focused and independent tests, Git/SQLite authority and no duplicate language-native implementations. \`SECURITY.md\` provides an initial private-disclosure route without fabricating contact details or promises.

The first public binary release is a separate acceptance gate, tracked by #83 and #113. Until a release exists and passes the full matrix, these are installed-from-source instructions and do not claim that binaries have been published.

Auto-ontology #88–#95 stays optional: concepts can annotate explanations via #94, but cannot establish an import, runtime behavior or safety to skip tests.

Contributor onboarding is verified through documented commands and installed CLI smoke, not a separately versioned example JSON schema.
