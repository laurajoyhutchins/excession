# Design reference: NVIDIA Auto Ontology

Upstream: https://github.com/NVIDIA/auto-ontology
Reviewed source revision: `fc43a82600e18b19d159c52023f54f5572f537ac`
License: Apache-2.0. This note documents design influences; no upstream source
code is copied or vendored into Excession.

## What the upstream actually does

NVIDIA Auto Ontology builds a semantic context layer on enterprise relational
catalogs, not source repositories. Its major stages are **propose**, **certify**,
and **answer**. Our relevant reference points are specific implementations,
not the product pitch:

| Source inspected in NVIDIA upstream | Design lesson |
| --- | --- |
| `auto_ontology/semantic/term_extractor.py` | Model-generated *names* and assignments are proposals with constrained input identities; its prompt expressly forbids deriving `IS_A`, `PART_OF`, or `ROLE` relations from a term label alone. |
| `auto_ontology/semantic/semantic_fk.py` | Declared relationships, suggested relationships, and fallback retrieval are different evidence classes. The implementation prunes proposed joins with no shared values and can abstain. |
| `auto_ontology/semantic/pipeline.py` | Parallel expensive extraction can coexist with a canonical, ordered commit phase, so concurrent execution need not make identity assignment nondeterministic. |
| `auto_ontology/dal/terms.py` | Certification is granular: term name and description flags are counted separately from related column and SQL attributes; the aggregate may be certified, partial, or pending. |
| `auto_ontology/retrieval/entity_coverage/models.py` and `mcp/auto_ontology_mcp/readiness.py` | Coverage and service readiness are distinct from finding or executing an answer. A populated catalog need not make a question answerable. |
| `mcp/auto_ontology_mcp/tools.py` | The agent-facing API is a small, explicit allow-list of meaning-oriented read operations, not a dump of every storage or mutation endpoint. |

The native architecture uses Python services, Postgres/pgvector, model services,
web UI and MCP. It is designed for data/SQL semantics, so **none of those runtime
choices should be treated as requirements for Excession**.

## Mapping into Excession

- **Source authority:** Git revision, tree, path, blob and exact SCIP symbol
  identity, with native compiler/indexer receipts.
- **Proposal authority:** deterministic Rust compiler producing `PROPOSED`
  claims with `UNTESTED` behavioral verification. A reciprocal file-reference
  pair alone does not establish shared architectural responsibility.
- **Review authority:** append-only, exact-concept-revision-scoped review
  events. Review of a *name* is not review of every membership or behavioral
  proof. Reviewer identity is not authenticated by a self-declared string.
- **Query authority:** bounded read-only Rust/SQLite queries. Explicit
  `UNKNOWN`, `PARTIAL`, `AMBIGUOUS` and `UNSUPPORTED`, preserving gaps
  and showing precisely which claims/receipts support an answer.
- **Action authority:** no automatic refactoring from ontology labels.
  Refactoring hypotheses and proof obligations retain their own evidence.
- **External tools:** official native SCIP indexers are allowed as observation
  producers. They are not replaced by a second homegrown Rust parser.

## Changes this reference warrants

1. Review names, membership edges and higher-level relationships **separately**,
   rather than upgrading an entire concept revision on a single approval.
2. Add independent predicate-specific evidence and explicit counterexamples
   before promoting a relationship. Fallback similarity/name matching is only
   candidate retrieval, not join or dependency proof.
3. Add question-level answerability/readiness tests grounded in source coverage,
   not just a successful snapshot publication.
4. Expose a small read-only agent vocabulary, derived from the existing Rust
   query surface only after exact-head integration acceptance.
5. Keep compilation, evidence queries, evaluation decisions, and source
   projections in Rust, using existing Git + SQLite infrastructure.
   Python scripts may remain as **external fixture/indexer drivers or
   independent cross-implementation oracles**, not as a second semantic
   engine or a permanent production evaluator.

## What the empirical results rule out

The current candidate rule simply maps reciprocal cross-file native SCIP
references into unreviewed source-pair concepts. In the 2026-11-11 held-out
native fixture run, **all 18 semantic file pairs matched existing native SCIP
refactor hypotheses**; none were a novel pair. No independent reviewers have
established architectural meaning. Model+normalized-batch serialized size was
22.49× the existing refactor reports in that fixture set.

That is a reason to keep the concept layer **opt-in**, not a reason to build
additional inference machinery or enable it by default. The next rule must
produce a useful, separately justified outcome under a preregistered oracle.

## Language and dependency policy

Rust is the single first-party semantic implementation and evaluator. Do not
port official third-party SCIP indexers to Rust merely for language purity.
Do not create a Python ontology service, LangChain pipeline, Postgres/pgvector
installation, extra database, or generic second graph layer. A temporary
fixture-runner script must not become a competing source of inference or
policy decisions.

Reference only. If any upstream code is later copied verbatim, follow
Apache-2.0 license and notice obligations, and record its exact origin.
