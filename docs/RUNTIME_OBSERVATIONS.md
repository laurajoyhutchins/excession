# Optional Python runtime import witnesses

This is an **explicit trusted-local tool**, not part of repository indexing.
It does not run untrusted code in a sandbox.

Run a script from a **clean, pinned SHA-1 Git checkout**:

```sh
python3 tools/python_import_observer.py \
  --repo /path/to/checkout --rev HEAD \
  --script scripts/test_entrypoint.py \
  --out /tmp/excession-runtime-witness.json \
  --trusted-local -- --my-script-argument
```

The output file must be outside the Git checkout and must not already exist.
The observer verifies the entrypoint against its pinned Git blob before
executing it. For each successfully resolved Python module observed through
standard import wrappers or a retained module-cache difference, it records:

- importer module, imported module, capture mechanism and observation count;
- normalized source origin, including `GIT_BOUND` blob mode/OID when the
  imported file is a tracked, unchanged regular Git source;
- explicit `UNTRACKED_REPO_PATH`, `EXTERNAL_TO_PINNED_REPO`,
  `GIT_BLOB_MISMATCH`, `NO_SOURCE_FILE`, and other unsupported
  origin statuses instead of manufacturing Git provenance;
- failed import attempts separately from positive witnesses;
- Python version and interpreter executable SHA-256;
- pinned Git commit and tree, exact entrypoint blob, execution status,
  elapsed time, selected Python audit-event counts, and a canonical report hash.

A target script's nonzero exit retains its partial observations. A dirty
tracked checkout, unpinned entrypoint, missing trust flag, or output inside
the source checkout refuses *before* executing target code.

## Guarantees and missing mechanisms

**Always `OBSERVED_RUNTIME_ONLY`.** This cannot prove every possible
runtime dependency. Standard Python import wrappers and `sys.modules` snapshots
are bypassable, and audit hooks are not a security boundary. Startup modules
are preloaded. Some import forms use cached modules without loader hooks;
other code mutates the module cache or loads native libraries directly.
Child processes, embedded interpreters, subprocesses, C extensions,
generated code, network-delivered code and malicious import hooks are not
fully monitored. A Git-bound target is a positive observation, not a claim
that no additional targets exist for other configurations.

We deliberately do **not** add module relationships to authoritative SQL
or promote a runtime witness to compiler-established truth. The language-
neutral semantic observation contract is being developed separately; this
Python-native receipt can later be mapped into that model without discarding
its original evidence and limitation fields.

## Reproducibility

No new Python packages, daemon, runner, network access or background agent are
required for the observer. It executes target code in-process and may expose
whatever capabilities and secrets the invoking user has. Run only with a
trusted script and environment.

The focused tests are:

```sh
python3 -m unittest discover -s tests -p test_python_import_observer.py -v
```