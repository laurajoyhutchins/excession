# Historical contributor-onboarding notes

> This page records an earlier implementation and release plan. For actual
> contributor setup, use [CONTRIBUTING](../../CONTRIBUTING.md),
> [development](../DEVELOPMENT.md), and [docs maintenance](../DOCUMENTATION.md).
> The current `excession.impact.v3` API selects source paths, not the
> obsolete `claim_id` parameter.

The root README now starts with \`discover\`, \`analyze\`, \`impact\` and replayable \`explain\` rather than an internal manually configured pipeline. The former impact contract required a `claim_id`; this historical requirement
was superseded by source-bound witness selection. \`--audit\` is explicit about what source replay does and does not verify.

\`CONTRIBUTING.md\` documents focused and independent tests, Git/SQLite authority and no duplicate language-native implementations. \`SECURITY.md\` provides an initial private-disclosure route without fabricating contact details or promises.

The first public binary release is a separate acceptance gate, tracked by #83 and #113. This was written before public release evidence existed. See the current
[releases](../RELEASE.md) for published artifacts and acceptance.

Auto-ontology #88–#95 stays optional: concepts can annotate explanations via #94, but cannot establish an import, runtime behavior or safety to skip tests.

Contributor onboarding is verified through documented commands and installed CLI smoke, not a separately versioned example JSON schema.
