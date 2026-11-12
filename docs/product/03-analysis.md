# PR 3: One-command repository analysis

Status: **scaffold only**. Contract: `excession.analysis.v1`.

## Workflow
`excession analyze . [--json]` is a proposed command, **not available yet**. It must coexist with the existing `excession analyze clang` subcommand rather than shadow or replace it.

Use the same Git/SQLite and language-native observation producers as `excession index`, `semantic scan`, `inspect evidence`, SCIP and configured graph. Aggregate receipt references, not a second store. Each relationship preserves its source/target Git binding and native evidence type. A valid report may be partial.

## Ontology seam
Auto-ontology PRs #88–#95 define proposed semantic snapshots and concept claims. The optional `ontology_links` here contains only opaque references to the future #94 annotation report. **Never use ontology membership to create import, call, compile, or runtime dependency edges.**

## Acceptance before implementation
- Independent Python/JS fixtures, missing analyzer and configuration tests; no implicit project-code execution.
- Exact-head output canonicalization and source replays.
- One-command fresh checkout scenario with cached evidence outside source.

Active schema tests reject silent proof promotion or ontology-edge promotion. Behavioral acceptance is skipped until implementation.

**Refs:** #79, #82, #84, #88, #94, #101.
