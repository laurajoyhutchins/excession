# Native dependency observation boundary

Excession authenticates source identity in Rust against pinned Git objects.
Language-native tools are observation producers, not independent authorities
for Git identity, historical evidence, or semantic completeness.

The public entry point is `excession evidence discover-native` with one
`clap` command grammar. The implementation uses `native_discovery.rs` and
the current bounded native-language adapter. `tools/excession_native.py` is
only a compatibility client; it is not an alternative evidence core.

## Current providers

| Language | Native observation | Boundary |
| --- | --- | --- |
| Go | Bounded `go list` | External modules, cgo and unsupported workspaces refuse |
| JavaScript / TypeScript | TypeScript compiler API and native module resolution | Requires the actual compiler API, not merely a `tsc` executable; unsupported or dynamic imports refuse |
| Python | Language-native import observation with explicit `--import-roots` | A declared source subset, not runtime I/O closure |
| Rust | **Unsupported by `discover-native`** | Use rust-analyzer SCIP for Rust symbols and references |

The prior `rustc --emit dep-info` dependency collector was deliberately
retired. Its old tests and implementation are preserved in historical Git
revisions, but current Excession must not silently restore it. The native
discovery command refuses Rust with an explicit rust-analyzer SCIP message.

## Evidence and trust

`--trusted-local true` is required before executing any native compiler or
language parser. This is **not** a sandbox or remote attestation. Input and
output bounds, temporary staging, source/blob verification and explicit
unsupported cases remain in effect.

A valid result contains a declared source closure and observed toolchain
assumptions, not proof of complete runtime dependencies or behavioral
equivalence. Historical transport compares declared input identities and
environment assumptions; it never inherits an old test pass as a new pass.

Use `excession evidence contract-plan`, `contract-compare`, and
`contract-audit` to bind observations and compare pinned revisions. In
CI, a TypeScript-specific test can skip only when the installed compiler API
is explicitly unavailable; the unsupported outcome must not be presented
as successful resolution.

Related material: [dependency discovery](DEPENDENCY_DISCOVERY.md),
[input contracts](INPUT_CONTRACTS.md), and [source evidence](EVIDENCE_HISTORY.md).
