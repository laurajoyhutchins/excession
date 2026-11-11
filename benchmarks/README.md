# Can Excession beat simple repository-navigation tools?

This directory is a **falsification benchmark**, not a demonstration. The initial executable slice tests one narrow question: at the *same indexed source revision and with exactly the same module/dependency edges*, what additional cost does Excession's deterministic dependency-view projection impose relative to direct graph traversal? Does it select the same source files?

This is **calibration only**. Query parity is not independent retrieval accuracy, and this batch does **not** measure whether an agent can solve a development task faster. The 24 questions are six mechanically specified graph queries against four h11 modules. They are not 24 independent engineering problems, and they must never be reported as such.

## Run the paired calibration

Build `core/target/debug/excession` first (`make cli`). From the repository root:

```sh
python3 benchmarks/paired.py run \
  --repo /path/to/clean/h11 \
  --bin ./core/target/debug/excession \
  --tasks benchmarks/calibration-h11.json \
  --out /tmp/excession-calibration-001 \
  --name h11 --roots h11,tests --rev HEAD \
  --repetitions 3 --random-seed 20261110
```

Use a clean checkout of the pinned h11 fixture. For an offline source, clone `core/fixtures/h11-crossrepo.bundle`; do not check the fixture into the benchmark output. Output must be a *new directory outside the source checkout*. The runner makes one indexed SQLite database using `excession index`, resolves the exact Git commit and tree, and tests both arms against that database. It invokes the graph baseline via a fresh Python process and the view arm through the installed Rust binary. The run writes:

- `metadata.json`: exact Git revision/tree, tool hash, task hash, index hash and bytes, measured indexing cost and environment limitations.
- `trials.jsonl`: one record per task, repetition and arm containing the returned source paths, source bytes, elapsed time, child CPU and explicit `oracle_correct` where reviewed gold exists.
- `summary.json`: paired path agreement, task count, cost medians and independently graded count.

The graph-only baseline is implemented with Python standard-library `sqlite3`, bounded breadth-first search, deduplicated paths and the exact same source and relationship tables used by the Excession view generator. It is a **simple baseline, not a deliberately broken one**. The graph worker includes Python interpreter startup and is not a pure Rust-vs-Rust microbenchmark. Comparisons must include cold indexing and amortized total cost. Real views perform Git/SQLite reconstruction and source authentication, so a latency difference cannot be interpreted as only a display-layout cost.

The task kinds are `focus`, `dependencies_direct`, `dependencies_all`, `dependents_direct`, `dependents_all`, and `tests` (the union of indexed test modules reachable through either direction). The view arm maps the existing canonical manifest roles to these questions. For performance comparability, it does **not** additionally replay or materialize each view. Evidence verification/replay must be measured as a separate, opt-in experiment with a plain Git/SQLite audit baseline.

Known measurement limits: Python child-process CPU and wall time are recorded, but peak RSS, allocator counts, actual model input tokens, human effort, cache effects and cost from a user's agent are **not** measured. Source bytes are retrieved after timing, from pinned Git objects. Calibration records with no independently reviewed oracle must retain `oracle_correct: null`, even if the two systems agree. Any empty outputs, unsupported cases or failed commands must remain visible; do not silently filter them out of the denominator.

Run the unit and real-binary integration smoke through `python3 -m unittest discover -s tests -p 'test_benchmarks.py'`. The real-binary test is skipped when the compiled executable or pinned source bundle is missing. The unit tests do not impersonate the Excession binary.

## Preregistered full comparison (not yet implemented)

For a held-out benchmark, compare these five task interfaces on exact Git commits, with identical task text, token budgets, and ground truth:

| Arm | Allowed instruments | Required control |
| --- | --- | --- |
| Keyword/file | `rg`, file globbing, file reads | Same revision and file-access budget |
| LSP/symbol | Language-server definitions/references/workspace symbols plus file reads | Pin server version and warmed index; retain unsupported languages |
| Graph-only | Shared `module`/`dependency` edges, arbitrary SQL/graph queries and file reads | **Exactly the Excession index**, no weaker extractor |
| Shell | Git, `find`, `rg`, `sed`, `awk`, `cat`, and read-only scripts | Agent may freely compose commands, not a crippled fixed grep pattern |
| Excession | `index`, `view`, `browse`, `view select/compose` and source reads | No private oracle, hidden answers, or Excession-only task hints |

Use **retrieval-only** and **agent engineering** modes separately. In retrieval mode, each interface returns source evidence under strict budgets (2K/8K/16K/32K *actual tokenizer counts*). Grade exact relevant files, precise evidence spans, recall and precision against *independently reviewed* gold. In agent mode, freeze the same coding agent/model/prompt and actual repository at a pinned commit, allow unrestricted use of each arm's documented read-only navigation tools, and grade final patches with hidden behavioral tests and human-auditable evidence. Repeat across randomized order and agent seeds. Do not let the agent access hidden test answers or future commits.

Task families: symbol location, behavioral discovery, change-impact prediction, cross-module investigation, real regression repair, and tampered-evidence verification. Deliberately include cases favorable to keyword search (exact names), language servers (symbols and references), graph traversal (multi-hop impact), and shell (quick exploratory code search), along with tasks where composable views plausibly help. Include cycles, aliases, dynamic imports, generated files, renamed paths, broken build systems and unsupported languages. Gold should come from independent behavioral tests, dynamic coverage, source review and explicit patch validation, **not** Excession's own index.

Use 24 calibration tasks to establish harness correctness and pilot cost, then a separately frozen 96-task held-out set across independent repositories. Keep repositories as statistical clusters. The primary end-point is correct agent tasks completed within a fixed time/token budget; secondary outcomes are evidence recall/precision, file bytes actually read, end-to-end time, process memory/CPU, tool calls, index updates and source-identity validation. Report CIs clustered by repository, the count of unsupported/refused tasks, and per-task traces. No cherry-picking a favorable cost metric after seeing results.

Preregistered rejection conditions: if Excession matches simple shell/keyword tools for a task but costs more, use the simpler tool; if it beats keyword retrieval but not LSP, do not credit derived views; if it matches graph-only, the extra view machinery has not shown value; if it beats graph-only on held-out patch success at comparable *total* cost, the view hypothesis has evidence. A proposed threshold is at least 10 percentage points more held-out tasks completed than the **strongest cheaper baseline**, with a repository-clustered confidence interval excluding zero; the threshold must be frozen before collecting scored results.

Report setup/indexing separately and give amortized costs for 1, 5, 20 and 100 tasks. Include cold and warm configurations and incremental commits. Measure standalone Git/SQLite verification against Excession's audit separately from navigation, since replay integrity is a different claim. Never portray faster query execution or identical source sets as evidence that Excession improves engineering outcomes.
