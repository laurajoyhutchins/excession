> **Historical implementation plan, not canonical product architecture.**
> This page records an earlier architecture-branch consolidation and
> history-reorganization effort. The current product decision is
> [investigation-first Excession](PRODUCT.md), with
> [canonical architecture](ARCHITECTURE.md),
> [evidence lifecycle](EVIDENCE_LIFECYCLE.md), and
> [ADR 0001](DECISIONS/0001-investigation-first.md).
> Any "target" or "implemented" language below is scoped to that historical
> branch and must be independently checked at today's revision.
>
> Retained below for provenance. It does **not** authorize a new
> graph-first roadmap or repository history rewrite.

# Historical module-ownership and history-reorganization plan

Status: executable target branch in progress. The source, evidence, and action module ownership moves below are implemented, while broader root-module consolidation and historical rewriting remain unfinished.

## Product invariant

Excession is a Git-grounded code-intelligence engine. It preserves a chain from immutable source or artifact identity to independent native observations, indexed evidence, analytical claims, and replayable decisions. Source correctness, observation correctness, and behavioral correctness are separate questions.

```text
Git object DAG + artifact byte identities
                 |
                 v
Native producers (SCIP, compiler, build, parser, runtime, scanner)
                 |
                 v
Authenticated receipts + typed SQLite evidence graph
                 |
                 v
Analysis: impact | explain | concepts | comparison | unknowns
                 |
                 v
Actions: agent context | tests | refactor | reconstruct | history
                 |
                 v
Independent verification + REA + build provenance
```

## Canonical ownership boundaries

| Domain | Owns | Must not duplicate |
| --- | --- | --- |
| source | Git commit/tree/blob identity, source path, exact diff, revision DAG | Native language resolution, speculative ontology |
| artifact | Byte digests, build provenance, mandatory REA links for artifact claims | Source identity for an unbound binary |
| evidence | Typed producer receipts, coverage facets, unknowns, immutable SQLite persistence, audit | Producer-specific semantics or alternate Git identity |
| observe | Source-bound native SCIP, Tree-sitter, Clang, Kbuild, runtime and scanner adapters | Handwritten replacements for complete native language semantics |
| analysis | Candidate relationships, impact witnesses, explanations, comparisons, semantic proposals | Proof of behavior or completeness from static edges |
| actions | Agent workflows, native tests, bounded refactoring and reconstruction, history plans | Mutating original user worktrees without explicit authorization |
| verification | Independent fixtures, replay, exact tree checks, artifacts, build receipts | Treating a passing test as universal correctness |
| surfaces | One Rust clap CLI, stable versioned JSON contracts, ergonomic docs | Parallel CLI grammar or historical research entry point |

Dependencies must flow from lower authority to derived views: actions depend on analysis and evidence, evidence refers to source/artifact IDs, and native producers own their interpretations. No cyclic dependency from an authoritative identity layer into an optional intelligence layer.

The implementation can retain a single Rust crate. The target is modular ownership, not a workspace full of mostly empty crates.

## Implemented on this architecture branch

- Git, source identity, exact change inventory, commit DAG planning and reconstruction replay live under core/src/source while preserving their existing public Rust module names.
- Git reconstruction evidence replay is under core/src/evidence.
- Isolated history synthesis and explicitly trusted local oracle execution are under core/src/actions.
- Verified work from PR #187 (Git reconstruction) and PR #207 (CLI/evidence identity consolidation) is integrated with preserved merge ancestry.
- History planning, replay audit, locally available ref inventory and exact target-tree comparison are implemented as read-only commands. Their output cannot authorize a rewrite.

## How the current code migrates

1. Establish an independent history inventory/replay API and a protected architecture branch. This branch begins with a read-only implementation, not a history rewrite.
2. Reconcile PR #207 into one source/evidence identity API and thin CLI dispatchers. Deduplicate serialized identifiers and source paths before moving files.
3. Consolidate related root modules under source, artifact, evidence, observe, analysis, actions and verification. Preserve public compatibility shims only where they have actual consumers. Move tests with ownership, not with historical PR boundaries.
4. Require REA and build provenance where an analysis makes artifact-level claims. Preserve explicit unsupported status otherwise.
5. Reconcile active PRs against the resulting graph. Integrate work only with a demonstrated owner, tested behavior, and a measured cost. Keep autonomous ontology and CPU-LLM experiments separate from the authority path.
6. Once the target tree is verified, construct a *separate* proposed Git ancestry. Compare final tree identity, original-to-new commit mapping, source/artifact receipts, merge semantics and independent results before any default-branch decision.

## Non-negotiable acceptance

- A normal command operates against pinned Git objects and is safe on dirty checkouts.
- New evidence is source/artifact bound with typed uncertainty. No candidate is silently promoted to truth.
- Tests demonstrate refusal on unsupported provenance, altered receipts, ambiguous identities and incomplete history ranges.
- Routine local checkpoint tests remain fast; expensive integrations run selectively, not on every commit.
- The final reorganized source tree must be byte-identical to the reviewed target tree. A tree match alone does not prove that rewritten intermediate history preserves semantics.
- No direct forced update of main from the history engine. No automatic deletion of historical archival refs, tags or open PR heads.

## Observable success criteria

A contributor can find code for a task, inspect the source/evidence that led to a candidate, select tests, validate a proposed edit, and audit a release or history plan using one Rust CLI. For every claim Excession can explain the authority, exact source identity, producer, relevant observations and outstanding obligations. Its own repository is an acceptance fixture, not a special-cased implementation.
