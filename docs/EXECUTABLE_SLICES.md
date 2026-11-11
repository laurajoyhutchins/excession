# Executable slices and conservative evidence carry

**Status: bounded Python-first trusted-local vertical slice.** This does not
establish semantic equivalence, secure isolation, remote attestation, or a
universal safe-to-skip-CI rule. It complements, but does not supersede, the
Rust/SQLite RelGit evidence authority or the existing read-only projections.

## The joined operation

A source-authenticated executable slice binds a selected test entrypoint to a
specific Git commit and the statically resolved local Python import closure.
Materialization copies exact Git blobs into a new read-only directory while
keeping original Python paths, so import rewriting is unnecessary for the
supported layout. An independent audit rederives the complete manifest and
checks every materialized file before trusted-local execution.

The execution receipt captures only a finite observed process exit and digests
of stdout/stderr, with Python version, view ID and commit. Receipts are hashed,
**not signed**, and must not be used as remote attestations.

A carry operation recomputes both the historical and target slice against Git
objects, compares path/blob closure identities, and returns one of:

| State | Meaning |
| --- | --- |
| `SOURCE_MATCH_ONLY` | Exact selected files and statically detected imports are unchanged; earlier observations remain historical, **not target behavior verification** |
| `REVERIFY_REQUIRED` | A selected blob, import resolution, selected asset or other closure member changed |
| Fresh target receipt | Optional explicit target-side execution reports `OBSERVED_PASS` / `OBSERVED_FAIL` only for the requested entrypoint; never universal compatibility |

Even an unchanged closure can be influenced by configuration, environment,
runtime patching, reflection, dependency injection or package installation. A
`SOURCE_MATCH_ONLY` verdict never promotes an earlier test to a target pass.

## Usage

Source checkout must be clean and at its pinned `HEAD`, and output files must
be **outside** the checkout.

```bash
# Generate a manifest for a plain Python entrypoint and explicit data assets.
python3 tools/excession_slice.py plan /src/repo tests/check.py \
  --asset tests/input.json > /tmp/base-slice.json

# Reconstruct only selected source from verified Git blobs.
python3 tools/excession_slice.py materialize /src/repo /tmp/base-slice.json /tmp/slice-base
python3 tools/excession_slice.py audit /src/repo /tmp/base-slice.json /tmp/slice-base

# This executes code; use only for source trusted to run on the current machine.
python3 tools/excession_slice.py run /src/repo /tmp/base-slice.json /tmp/slice-base \
  --trusted-local > /tmp/base-receipt.json

# After branch integration, in a checkout whose HEAD is the integrated commit:
python3 tools/excession_slice.py carry /src/integrated \
  /tmp/base-slice.json /tmp/base-receipt.json

# Optional: make a fresh target slice and run its entrypoint, not merely carry.
python3 tools/excession_slice.py carry /src/integrated \
  /tmp/base-slice.json /tmp/base-receipt.json --replay --trusted-local
```

## Scope and refusals

- Python source located at repository root, with ordinary packages containing
  `__init__.py`; relative and absolute static imports are conservatively
  resolved. `from ... import name` requires a local module or a statically
  declared top-level export. Namespace packages, external libraries and star
  imports are unsupported. Stdlib imports use the executing Python's stdlib
  inventory, not a complete environment lock.
- Dynamic imports and common dynamic code-loading constructs are refused.
  This static analysis cannot detect all possible reflection, side effects,
  runtime file reads or implicit dependencies. Explicit data assets can be
  added, but their completeness is a caller responsibility.
- Pinned SHA-1 Git revisions only, no dirty checkout, symlinks, executable
  file modes, missing modules, ambiguous source paths, or mutation of original
  source. Maximum 512 files and 32 MiB, plus bounded native execution time and
  output. Materialized files are read-only and checked again before execution.
- Execution invokes the current Python interpreter in isolated Python mode,
  but **is not an operating-system sandbox**. The `--trusted-local` gate is
  mandatory. Never run unknown or hostile repository code on the shared host.
- Fresh test results prove only that the selected command passed or failed in
  that finite environment. They do not establish equivalence of the entire
  source or that no other tests need to run.
- Hash-only receipts are forgeable by an actor controlling the receipt file.
  Before trusting external workers, add signed environment attestation,
  isolation controls, and a Rust-authenticated evidence import path.

## Verification

```bash
python3 -m unittest discover -s tests -p 'test_executable_slices.py' -v
```

The offline tests include a two-agent nonoverlapping merge, modified-dependency
reverification, a real changed-behavior test failure, manifest/receipt tampering,
dirty input, symlinks, unresolved exports, dynamic import and external package
refusals. The source used in these tests is generated from small real Git
repositories rather than mocks of Git identity.

## Next contract increment

This initial CLI is explicitly a language-native adapter. Generalize its
manifest into the existing Rust source/claim schema and make Rust the
cross-language authority for indexing, claim-scoped closure validity and signed
receipt ingestion. Python retains ownership of import semantics. Then add
controlled runtime import/file traces in an isolated worker, and test whether
incremental reruns save time over full CI on several real repositories.
