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
