# Executable slices and conservative evidence carry

**Status: bounded Python adapter with Rust/SQLite evidence ingestion.** This does
not establish semantic equivalence, remote attestation, or a universal safe-to-skip-CI
rule. A Linux namespace worker is available for explicitly approved local execution,
with no silent fallback. Source and SQL authenticity are distinct from executor authenticity.

## The joined operation

A source-authenticated executable slice binds a selected test entrypoint to a
specific Git commit and the statically resolved local Python import closure.
Materialization copies exact Git blobs into a new read-only directory while
keeping original Python paths, so import rewriting is unnecessary for the
supported layout. A local audit rederives the complete manifest and
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
  --trusted-local --isolated-linux > /tmp/base-receipt.json

# Bind the finite receipt to Git objects in Rust-owned SQLite, then replay the audit.
excession slice ingest --repo /src/repo --manifest /tmp/base-slice.json \
  --receipt /tmp/base-receipt.json --db /tmp/evidence.sqlite
excession slice audit --repo /src/repo --manifest /tmp/base-slice.json \
  --receipt /tmp/base-receipt.json --db /tmp/evidence.sqlite

# After branch integration, in a checkout whose HEAD is the integrated commit:
python3 tools/excession_slice.py carry /src/integrated \
  /tmp/base-slice.json /tmp/base-receipt.json

# Optional: make a fresh target slice and run its entrypoint, not merely carry.
python3 tools/excession_slice.py carry /src/integrated \
  /tmp/base-slice.json /tmp/base-receipt.json --replay --trusted-local --isolated-linux
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
- Execution always requires `--trusted-local`. Without `--isolated-linux`,
  Python `-I` alone **is not an OS sandbox**. For supported Linux machines,
  the opt-in worker uses a new user/mount/PID/network/IPC/UTS namespace, an
  isolated chroot, a read-only `/usr` and source bind mount, a small tmpfs,
  dropped capabilities, `no_new_privs`, and bounded CPU/address space. Failure
  to enter the worker refuses execution, never falling back to the host.
  The worker is tested on one Linux host; these controls are **not a remotely
  trusted attestation or a guarantee of containment**. Do not treat this as
  sufficient to run adversarial code in a credential-bearing environment.
- Fresh test results prove only that the selected command passed or failed in
  that finite environment. They do not establish equivalence of the entire
  source or that no other tests need to run.
- The Rust core's `slice-evidence-ingest`/`slice-evidence-audit` and the public
  `excession slice ingest|audit` commands validate canonical manifest/receipt
  digests, exact tracked Git source blobs and modes, limits, and SQLite rows.
  A second audit recomputes everything from the original inputs and compares
  exact stored rows, refusing tampering or duplicate ingestion. Rust does *not*
  independently rederive Python's import semantics: that remains an adapter
  claim, with dynamic imports and external state explicitly unsupported.
- Hash-only Python receipts are forgeable by actors controlling the receipt
  bytes. `LOCAL_LINUX_NAMESPACES_UNATTESTED` records a local policy selection,
  **not cryptographic proof of the worker**. Signed workload identity, pinned
  toolchains and trustworthy remote-worker attestations are still required
  before cross-agent or distributed execution claims can be trusted.

## Verification

```bash
python3 -m unittest discover -s tests -p 'test_executable_slices.py' -v
# Optional on a Linux host permitting user/mount namespaces:
EXCESSION_RUN_NAMESPACE_TESTS=1 python3 -m unittest discover -s tests -p 'test_executable_slices.py' -v
cargo test --offline --manifest-path core/Cargo.toml slice_evidence
```

The offline tests include a two-agent nonoverlapping merge, modified-dependency
reverification, a real changed-behavior test failure, manifest/receipt tampering,
dirty input, symlinks, unresolved exports, dynamic import and external package
refusals. The source used in these tests is generated from small real Git
repositories rather than mocks of Git identity.

## Next contract increment

Python remains the language-native import planner; Rust now owns storage,
source integrity validation, and independent SQL replay. The next decisive
contract is **runtime import and file-read observation inside an attested worker**, 
followed by claim-scoped evidence carry and controlled comparisons against full
CI on multiple genuine repositories. Until that is demonstrated,
`SOURCE_MATCH_ONLY` is not permission to skip tests.
