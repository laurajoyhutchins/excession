# Excession architecture

**Product:** Git-grounded, auditable code intelligence.
**Version:** 0.1.x. **Source authority:** Git objects; Rust and SQLite maintain
indexed relationships and imported evidence.
**Scope:** source-bound analysis at pinned revisions, with explicit coverage
for each observation producer.

## 1. Code-intelligence foundation

Excession assembles **inspectable evidence about code** without replacing the
source tree or the tools that understand each programming language. A repository
revision stays the common reference point:

```text
Pinned Git commit, tree, paths and blobs
                |
                v
Language-native parsers, SCIP indexers, compiler inputs, scanner reports
                |
                v
Source binding + typed observations (Rust CLI, SQLite, JSON contracts)
                |
                v
Queries, refactoring candidates, repository views, replay and audits
```

The stages have distinct responsibilities:

- **Git** supplies the source identity: a commit, tree, path and blob.
  The current source core supports SHA-1 Git object repositories.
- **Producers** observe syntax, static dependencies, symbol references,
  compiler-declared inputs, runtime witnesses or scanner findings. Their
  meaning and coverage remain producer-specific.
- **Excession** checks source bindings and records relationships in SQLite
  or versioned reports. It does not invent edges between incompatible
  evidence types.
- **Views and queries** reorganize or select recorded evidence while retaining
  original source identities.
- **Audits** replay a supported observation or query against its stated
  inputs. Replay establishes reproducibility and source consistency for that
  contract, not completeness of an analyzer or safety of a code change.

### Implemented interfaces

| Capability | Mechanism | Current boundary |
| --- | --- | --- |
| Git-backed module index | Rust/SQLite with Python/JavaScript extraction | Bounded static relations in selected roots |
| Repository views | Virtual composition, browsing, optional read-only materialization | Source-linked layouts, not executable reorganizations |
| Syntax/semantic observations | Optional Tree-sitter and language-aware resolvers | Reported candidates and unresolved cases |
| Symbol evidence | SCIP supplied by language-native indexers | Git-checked occurrences and reference hypotheses |
| Source inspection | `inspect evidence` and native input comparison | Distinct static, compiler and runtime observations |
| Scanner evidence | CodeQL SARIF and Semgrep import/audit | Source-bound reports, not confirmed vulnerabilities |

These interfaces have different coverage and authority. See
[CLI](CLI.md), [repository views](REPOSITORY_VIEWS.md),
[semantic contracts](SEMANTIC_CONTRACTS.md), [evidence queries](EVIDENCE_QUERY.md),
and [refactoring](REFACTORING.md).

### Public command surface

The installed `excession` Rust executable provides the Git/SQLite CLI.
`excession research ...` retains historical reproducibility operations without
introducing a second executable. Optional trusted-local scanner execution is
not a sandbox. Python tools remain optional adapters and reference checks.

## 2. Research application: vulnerability discovery

The evidence layer can support controlled security investigations. The
unproven hypothesis is that joining source relationships and revision history
with static analysis and native fuzzing produces *more confirmed findings*
than static analysis and fuzzing alone, at comparable resource cost.

**Hypothesis:** `confirmed(Excession + static analysis + fuzzing) > confirmed(static analysis + fuzzing)`
on a preregistered held-out corpus. No demonstrated autonomous discovery
superiority follows from the existing ingestion or replay features.

### Proposed security evaluation pipeline

The following is an **experiment design**, not a currently delivered end-to-end
vulnerability discovery system:

```text
Pinned revisions, frozen trial and baseline scanner/fuzzer outputs
                         |
                         v
CodeQL / Semgrep + optional language-native evidence
                         |
                         v
Excession source binding, normalized candidates, SQLite provenance
                         |
                         v
Isolated native tests, controls and observed traces (research integration)
                         |
                         v
Independent validation and frozen findings
                         |
                         v
Post-freeze OSV / advisory matching and baseline comparison
```

CodeQL's dataflow engine, Semgrep's matcher and native fuzzers remain external.
An imported warning is not a verified vulnerability; confirmation requires
independent behavioral and impact evidence. The discovery protocol and
security controls below apply to this research use case.

## 3. Existing engines and interface contracts

| Interface | Input | Output | Current support |
|---|---|---|---|
| CodeQL | existing language-specific DB + query suite | SARIF 2.1.0 | CLI invocation, Rust report validation and import |
| Semgrep | clean source checkout + local pinned rules | JSON `results` | CLI invocation, Rust report validation and import |
| Joern | source checkout and CPG | candidate edges with provenance | planned; no working adapter |
| Go fuzzing / libFuzzer / AFL++ | explicit trusted native harness + seed corpus | bounded native traces/crashes | research integration planned; existing RelGit native pilot retained |
| OSV / GitHub advisories | frozen finding IDs + exact package/revision | CVE match candidates | planned post-freeze evaluation, not part of discovery |
| Excession | exact Git commit and tool output | canonical candidate and audit receipts | implemented |

Tool references: CodeQL SARIF (<https://docs.github.com/en/code-security/reference/code-scanning/sarif-files/sarif-support>),
Semgrep JSON (<https://github.com/semgrep/semgrep-interfaces/blob/main/semgrep_output_v1.jsonschema>),
Go fuzz (<https://go.dev/doc/security/fuzz/>), OSV (<https://osv.dev/docs/>).

The scanner runner does not build CodeQL databases, install query packs, or
execute untrusted source. CodeQL databases must be created by a separately
reviewed and isolated build. Reports must use plain repository-relative source
paths; ambiguous/encoded/absolute paths are refused rather than guessed.

## 4. Authoritative identities

* Source authority: pinned Git SHA-1 commit + path + blob SHA. The current
  Rust core explicitly rejects SHA-256 Git object-format repositories.
* Analysis identity: SHA-256 of the original scanner report bytes, tool name,
  exact source commit. A report is **not** a signed tool attestation.
* Candidate identity: SHA-256 of engine, rule, path/line logical anchor, blob,
  and canonical raw finding. Stable logical keys permit preliminary revision
  comparison, but code moves and line shifts invalidate naive comparisons.
* Evidence authority: SQLite foreign-keyed rows and an independent Rust replay
  comparing every stored row to fresh Git and input report data.
* Native trace identity: separate bounded execution receipt; never inferred
  from static scanner output.
* Advisory identity: post-freeze, matched through precise affected version
  ranges and behavior, not only substring matches in CVE descriptions.

A database hash alone does not authenticate remote execution or detect deliberate
replacement of the entire database and its source inputs. Remote attestations and
sandbox identity are separately required before trusting third-party agents.

## 5. Relational state and proof obligations

First implementation tables:

- `security_ingest_run(run_id, source_commit, report_sha256, engine, candidate_count, flow_step_count, status)`
- `security_ingest_candidate(run_id, candidate_id, logical_key, source_path,
  source_blob, line, rule_id, raw_evidence)`
- `security_ingest_flow_step(run_id, candidate_id, flow_index, thread_index, step_index,
  source_path, source_blob, line)` (SARIF code-flow location chain)

Implemented status is exclusively `UNVERIFIED_STATIC_CANDIDATES`.

Future evidence tables should be *additive* and state-typed:

- `security_flow(source, transformations, sink, analyzer_receipt)`
- `security_invariant(id, precondition, safe_property, scope)`
- `security_execution(target_commit, runtime_digest, harness_digest,
  input_digest, observed_trace_digest, controls)`
- `security_verdict(state, reachability, impact, reviewer)`
- `security_advisory_match(post_freeze_only, source, id, affected_versions)`

Allowed progression: `CANDIDATE -> REPRODUCED -> REACHABLE -> IMPACT_CONFIRMED`.
A separate `DISCONFIRMED` and `UNSUPPORTED` terminal outcome must remain in
counts. *No inference from a lower state to a higher state.* A CVE ID is
metadata only after a post-freeze match, not a security-verdict state.

## 6. Trusted-local ingestion flow (implemented)

1. Require an existing clean SHA-1 Git checkout; record `HEAD`.
2. Run a named installed scanner without a shell, at bounded time, using
   explicitly supplied local CodeQL queries or Semgrep rules.
3. Require scanner report outside target checkout and capped at 16 MiB.
4. Parse SARIF 2.1.0 or Semgrep JSON in Rust using strict duplicate-key-rejecting
   JSON parser. Refuse unsupported URI encodings and untracked paths.
5. Resolve each candidate to a tracked regular file blob and validate line span.
6. Bind every supported CodeQL path-query step to a tracked source blob and
   store ordered thread-flow relations in the same SQLite transaction.
7. Insert canonical evidence into SQLite in a single transaction; no CVE label
   is applied.
8. Independently rerun source validation and report parsing; verify exact
   rows, row counts, statuses and FK constraints. Fail on forged extra rows.

The importer deliberately supports a narrow subset: one CodeQL run,
primary locations with start lines, `tool.driver.name=CodeQL`, and Semgrep
`results/check_id/path/start.line`. More complicated tool reports must be
explicitly supported rather than silently truncated.

## 7. Discovery and replay protocol

**Before reading advisories:** Select repositories by publicly articulated
neutral criteria (language, size, fixed historical date, supported analyzers),
freeze commit IDs, invariant families, and hashes of rule packs, and precommit control cases and budgets.

**Discovery:** Run CodeQL baseline, Semgrep, and designated native fuzz tests;
freeze the raw outputs before joining any advisory data. Select representative
counterexamples without guidance from the known patches. Record unsupported
cases and scanner failures in the denominator.

**Validation:** Execute the same input on vulnerable and patched revisions,
ideally through the real request path. Require negative/positive controls,
sandboxed runtime evidence, and effect/impact confirmation. Rerun an independent
audit from a clean checkout.

**Scoring:** Only after freezing, consult CVE/OSV advisories and patch histories.
Count true positives only with behavior and affected-version agreement. Compare
Excession-enhanced output against CodeQL-only + fuzz-only baselines at similar cost.

## 8. Security controls

- Analyze only projects and systems authorized for security research.
- No untrusted user-source execution on the shared host; use isolated workers
  with no credentials, egress restrictions, CPU/memory/time ceilings and cleanup.
- Do not upload potentially sensitive reports to public GitHub alerts by default.
- Never pass credentials in scanner command-line arguments or reports.
- Prefer pinned local rule packs; avoid auto-fetch or metrics where possible.
- Refuse dirty checkouts, symlink source targets, invalid paths, misleading
  scanner format, unexpected database rows, and missing evidence.
- Treat analysis tool outputs and their messages as adversarial data.
- Coordinate disclosure for any actionable new finding; no automatic CVE claim.

## 9. Compute and CI

Local commit check (<30s target): Rust formatting/strict Clippy, core unit
and importer fixture tests, basic Python unittest. It should not trigger a
full Kubernetes build or an unbounded fuzzer.

Cloud Build/manual deep check: fetch exact source revision, construct CodeQL
DB, run code scanning, seeded fuzzing in isolation, audit and cost receipts.
Explicit environment version-pins required. No GitHub Actions minutes assumed.

## 10. Acceptance criteria and implementation status

**Delivered now:** new local git repo; Rust SARIF/Semgrep candidate ingest and
audit; runner for local CodeQL/Semgrep; label-free benchmark protocol freeze;
preliminary cross-revision candidate comparison; offline fixture and tampering
regressions; inherited RelGit v0.19 Git/SQLite/native pilots.

**Still required for demonstrated CVE-discovery superiority:** real CodeQL and
Semgrep installs; trusted source-built CodeQL DB; native fuzz harness generator;
replayable authorization impact; OSV post-freeze range matcher; blind historical
benchmark; measurement against standalone scanners; remote attestations.

## 11. Rejection policy and design choices

- No general-purpose CPG rewrite in Rust. Consume Joern/CodeQL output.
- No automatic acceptance of "fixed" from absent static findings.
- No vulnerability claim from static patterns alone.
- No speculative aggregation of three wrappers over one transport bug as
  three vulnerabilities.
- No generic agent orchestration or always-on SaaS dashboard.
- Extend native adapters only when a real bounded test case requires them.

**Next decisive experiment:** a preregistered Go repository benchmark comparing
CodeQL-only + native fuzzing against the same evidence joined through Excession.
