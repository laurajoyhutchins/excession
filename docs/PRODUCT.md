# Excession: product contract

**Decision:** Excession is an evidence-driven codebase-understanding and
transformation system. Its primary unit of work is an **investigation**, not
a symbol lookup or graph query. A transformation consumes an investigation
and states what behavior must survive the change. Verification reports what
the available evidence does and does not establish.

This is the **canonical product direction**. It does not claim that an
end-to-end `excession investigate` or `excession transform` command already
exists. See [current capabilities](#current-versus-target-capabilities).

## User outcomes

1. **Understand:** answer a concrete software question by locating the
   governing implementation, reconstructing relevant behavior, relating
   components and tests, and naming the remaining uncertainty.
2. **Transform:** describe a better structure, identify edit boundaries and
   preservation obligations, use appropriate language-native change tools,
   and verify the resulting program against those obligations.
3. **Reuse:** carry source-bound findings across investigations and invalidate
   them selectively when their dependencies change.

Excession's output is a smaller, falsifiable mental model of a system,
not a larger inventory of indexed relationships. The explanation must
differentiate *source evidence*, *producer observation*, *derived claim*,
and *independent test or runtime observation*.

## A single workflow

```text
Question + repository + exact revision
  -> bounded evidence discovery
  -> inspection and competing hypotheses
  -> behavioral claims + unknowns
  -> investigation report
  -> optional change proposal + preservation obligations
  -> native edits in isolated candidate revision
  -> independent checks and differential observations
  -> verification report + updated claims
```

The investigation can loop: missing evidence prompts a targeted source read,
build inspection, test, or runtime trace. It must never turn an unexamined
relationship into a behavioral fact.

## System boundaries

| Owner | Responsible for | Not responsible for |
| --- | --- | --- |
| Git | Authoritative tracked source identity and revision history | Proof of behavior |
| SCIP, compilers, language servers, parsers | Observations in their supported domains | General Excession explanations |
| Tests, builds, traces, independent oracles | Scoped execution observations and receipts | Universal correctness from one green result |
| Excession evidence layer | Typed provenance, identities, coverage, replay, invalidation | Redoing native indexing algorithms |
| Excession investigation engine | Evidence selection, explanatory claims, alternatives, unknowns | Unqualified proof from static edges |
| Excession transformation workflow | Preservation contract, edit boundaries, impact and verification plan | Rewriting languages in a universal internal parser |

The [evidence lifecycle](EVIDENCE_LIFECYCLE.md) uses generation, ingestion,
storage, transformation, and serving as useful *data-system responsibilities*.
This is an architectural adaptation of the lifecycle discussed by Joe Reis
and Matt Housley, not a mandate to build a warehouse or orchestrator.

## Current versus target capabilities

| Capability | Current main-line support | Target |
| --- | --- | --- |
| Source discovery | `agent locate` ranks Git-bound candidates and a bounded reading plan; `investigation start/claim/show` persists source-bound notes | Question-led implementation discovery with follow-up decisions |
| Context | `agent context` exposes bounded source/evidence/test candidates | Behavior-centered, traceable explanation |
| Impact | `impact` and `agent verify` report candidates and obligations | Impact scoped to specific preservation claims |
| Refactoring | `refactor`/`improve` provide native-edit paths; `investigation plan` records preservation obligations | Investigation-driven, cross-module transformation |
| Verification | Evidence replay and native receipts; `investigation verify` persists `agent verify` with open obligations | Claim-by-claim behavioral preservation report |
| Serving | CLI, local stdio MCP, GitHub Actions artifacts | First-class investigation/transform/verify agent interface |

The ledger is a first implemented artifact, **not** the target automated
investigation engine or behavioral equivalence checker. Do not advertise
the target column as shipped. Existing commands and
wire schemas are documented in [CLI](CLI.md), [agent CLI](AGENT_CLI.md),
[refactoring technical reference](REFACTORING.md), and [MCP](MCP.md).
These remain supported implementation surfaces, not competing product
definitions.

## Non-goals

- Competing with SCIP on symbol coverage merely to increase indexed edges.
- Constructing the entire program graph before a question is asked.
- Inventing runtime calls, control flow, semantic equivalence, test completeness,
  or performance guarantees from static relationships.
- Building another universal language parser or source rewrite engine.
- Mutating user worktrees or executing untrusted builds as a side effect of
  read-only investigation.
- Maintaining a second Git authority or treating a cache as verified source.

## Acceptance

Product success is measured by independently graded answers and successful
behavior-preserving refactorings on unfamiliar repositories, including
negative cases. Coverage, index size, and retrieval speed are diagnostics.
See [cutover inventory](CUTOVER.md), [evaluation](EVALUATION.md),
[investigation contract](INVESTIGATIONS.md),
[transformation contract](TRANSFORMATIONS.md), and
[verification](VERIFICATION.md).
