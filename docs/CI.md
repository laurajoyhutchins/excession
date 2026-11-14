# Excession CI execution policy

## Execution tiers

| Tier | Trigger | Responsibility |
| --- | --- | --- |
| Local checkpoint | Developer or agent | Narrow tests, formatting and diff check; warm-target goal under 30 seconds |
| PR admission | Every PR and main push | Verify: Rust format, Clippy, native tests, Node and Python checks, agent and Git-history audit |
| Focused evidence | PRs changing producer-owned files or manual dispatch | SCIP, Clang, Kbuild, parsers, configured graphs, external fixtures and source-bound refactoring |
| Deep and release | Manual, scheduled advisory jobs, release branches, explicit labels or version tags | Large corpora, historical studies, full portability and independently verified published artifacts |

The shared Rust CLI and lib.rs are integration plumbing, not evidence that every language producer changed. Focused workflows must use their own implementation, fixture and contract paths, not shared CLI or library-root catchall paths. A change affecting multiple producers should run the relevant workflows explicitly. The broad Verify gate remains required on every pull request.

PR concurrency is grouped by workflow and PR number. Superseding pushes cancel stale jobs, but manual, scheduled and tag-related evidence should not be cancelled. A success at an older SHA does not verify a newer head.

## Release gate remains mandatory

Ordinary feature PRs do not run full portability. Add the ci:full-portability label to any PR for an immediate full-matrix run, use a release/ branch, or dispatch Portable CLI manually. Version tags v* always run the full matrix, musl and extracted-archive acceptance. Publication requires all three gates, verified source manifest and archive digests, and refusal to overwrite an existing release.

The full platform and upstream acceptance tests are not replaced by a passing general Verify job.

## Deep tests and evidence

The named native workflows retain manual dispatch. Run selected Clang, SCIP, Kbuild, Linux inventory, historical refactoring, independent query oracles and benchmarks for changes affecting those semantics. Self-analysis remains weekly/advisory. Full Linux sweep, kernel probe and relational benchmarks remain manual. Never count a skipped suite as a passing observation.

## Regression testing

The standard-library-only tests/test_ci_topology.py runs before expensive dependency installation. It prevents focused workflows from regaining shared root triggers and ensures the release tag gate is retained. This tests the workflow policy, not the full GitHub Actions parser. Exact-head GitHub runs are still required.

The normal PR checkout is shallow for cost. The history planner must report SHALLOW rather than complete ancestry. History reconstruction requires an independently verified full checkout and source evidence.

Do not change required branch-protection checks without separately confirming the rules from an authorized GitHub account.
