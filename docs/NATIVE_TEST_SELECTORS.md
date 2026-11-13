# Native test selectors

Excession delegates test selection to tools that already know the language,
framework and runtime. It does **not** invent a second test dependency graph.

Two native providers are initially implemented:

| Provider | Native action | What Excession records |
| --- | --- | --- |
| Jest | list related tests with --findRelatedTests --listTests --json | source-bound tracked test file paths |
| pytest-testmon | execute the cached, coverage-driven selection with --testmon --testmon-forceselect | execution result and an unattested agent-compatible test receipt |

## Trust and source identity

Both integrations are **explicit trusted-local execution**. They invoke native
project tools and configurations, which can execute arbitrary code. They never
run merely because Excession recognizes a language. Use an isolated worker for
untrusted agent patches.

The candidate Git commit must be checked out at HEAD and have no tracked
working-tree modifications. A distinct base Git commit must be provided.
Renames/deletions refuse rather than silently disappearing from the selection.
For Jest, selected files must be absolute paths resolving to regular tracked
source files within the requested candidate Git repository.

No provider result proves that other tests can safely be skipped. An empty
selection is an unproven result, not a successful completeness guarantee.
All provider observations remain scoped to the exact candidate commit/tree.

## Jest

Install project dependencies first (Excession does not download packages),
then execute from a trusted candidate checkout:

~~~sh
excession agent select-tests --repo /work/js-repo \
  --base BASE_SHA --candidate HEAD_SHA \
  --provider jest --trusted-local true
~~~

By default the tool is the checkout-local node_modules/.bin/jest. An
explicit --tool path may point to an installed Jest binary. Excession
derives exact changed tracked paths from Git and passes those paths to
Jest's --findRelatedTests --listTests --json. Jest owns configuration,
module resolution, transitive reachability and test selection. Excession
only validates and source-binds the returned test file paths.

Jest listing is not test execution. The report has no passing-test receipt.

## pytest-testmon

Build a **real executed test coverage baseline** before incrementally selecting:

~~~sh
# On trusted source with the plugin already installed
python3 -m pytest --testmon --testmon-noselect
# Commit / checkout the candidate revision (the .testmondata history persists)
excession agent select-tests --repo /work/python-repo \
  --base BASE_SHA --candidate HEAD_SHA \
  --provider pytest-testmon --trusted-local true
~~~

This command invokes Python's pytest-testmon plugin, which selects and
**executes** the affected tests. It requires an existing .testmondata file.
Pass --tool /path/to/venv/bin/python if the plugin is installed in another
interpreter. An optional --focus tests/file.py limits pytest collection;
the plugin still owns the selection. Passing a scope does not prove that
other tests are unaffected.

The provider also requires the installed pytest-json-report plugin. The JSON
report supplies the native executed-test count and test node IDs. If the plugin
deselects every test, Excession returns EMPTY_UNPROVEN and emits **no PASS
receipt**, even when pytest exits 0. This prevents an empty-selection false
green. The native temporary report is created outside the checkout and removed.

The result includes execution_receipts with schema
excession.execution-receipts.v1. To verify the same candidate revision
without re-executing the selection:

~~~sh
excession agent select-tests ... > /tmp/test-selection.json
jq '.execution_receipts' /tmp/test-selection.json > /tmp/receipts.json
excession agent verify --repo /work/python-repo \
  --base BASE_SHA --candidate HEAD_SHA --receipts /tmp/receipts.json
~~~

Excession verifies revision identity and receipt structure, **not**
the authenticity of a native execution process. It retains open merge,
test coverage and attestation obligations. Do not claim safe test elision
from a selector's subset, even when the native tool reports PASS.

## Boundaries

- Executed providers are opted in; untrusted code must run in an isolated
  worker with resource limits.
- Excession does not automatically install or update pytest-testmon or Jest.
- The Jest result lists files, not individual test cases.
- pytest-testmon's output records executed outcome, not a fabricated list of
  selected test names. Its own database and plugin decide the subset.
- A source-bound provider receipt is not a cryptographic attestation.
- More providers can use the same native selection contract without adding
  homegrown dependency or language-specific test-selection heuristics.


The output contract is [native-test-selector.v1.schema.json](../contracts/agent/native-test-selector.v1.schema.json).
Native stdout/stderr SHA-256 digests and native exit status are retained.
Excession checks that tracked source still matches the requested Git commit
**after** the selector exits, since installed project tools may mutate the tree.
A testmon --focus file must itself belong to the pinned Git revision.


## Everyday programmer and agent workflow

From a checkout with committed changes, use **\`excession tests\`**.
It plans without running project code, finds a Git base using a merge base
against a local main branch where available (falling back to the first parent),
and detects installed Jest or Python pytest-testmon. It shows missing setup,
a dirty checkout, unsupported change kinds, or ambiguous frameworks explicitly.

~~~sh
excession tests
excession tests --run
excession tests --run --verify --json
excession tests --base origin/main --provider jest --run
~~~

\`--run\` is explicit authorization to execute locally installed project
tools. For untrusted coding-agent revisions, use a disposable isolated worker
first. A dirty tracked checkout cannot be passed off as a committed revision.
The existing \`agent select-tests\` keeps the stricter explicit
\`--trusted-local true\` interface and its v1 JSON contract.

The ordinary workflow reports a concise summary, with \`--json\` for agents
and automation. Exit codes: 0 for a completed passing run or successful plan,
1 for genuine test failures, 2 for refusals or tool errors, and 3 for an
inconclusive executed selection. A plan marked BLOCKED has no test execution
and is not a successful verification claim.

For Jest, the workflow invokes native related-test selection and then runs
those files using Jest's native \`--runTestsByPath\` execution. Test counts,
native digests, and a source-bound execution receipt are recorded. For
pytest-testmon, the native execution report and receipt already come from the
selector. \`--verify\` transfers these receipts to Excession automatically:
no \`jq\` script, manual hash lookup, or temporary file management is needed.
An accepted receipt is still unattested; merge readiness remains unproven.

If both frameworks are present, pass \`--provider jest\` or
\`--provider pytest-testmon\`. To analyze a non-HEAD candidate, use a
checkout of that candidate; the CLI will not execute mismatched code.


Provider detection also checks Jest declarations in a tracked root-level
\`package.json\`, not only an installed binary or Jest config. Such a project
can therefore produce a useful *blocked* plan before dependencies are installed.
The displayed next action distinguishes an already-ready checkout from missing
native prerequisites.


For programmatic agents, the test plan also supplies \`next_argv\`: a complete
argument vector containing absolute source identities, exact base/candidate
commits, provider and focus. Its final \`--json\` argument ensures the next
execution produces machine-readable evidence. Unlike re-running automatic
discovery, this preserves what the agent approved during planning. Human
output includes the shell-quoted equivalent \`next_command\`.


If \`--verify\` cannot initialize an optional Excession analyzer (for example,
a standalone Rust binary without the JavaScript sidecar), the workflow keeps
the native test report and returns \`verification.status=UNAVAILABLE\` with
\`ready_to_merge=false\`. A native PASS then has overall status INCONCLUSIVE
and exit code 3 rather than falsely claiming verification. A real native
test failure remains FAIL with exit code 1.
