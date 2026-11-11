# Python native input discovery, explicitly configured

The historical dependency declaration path supports Python on the **same
Rust-owned Git authority** used for Go, Rust and TypeScript. Python source is
parsed with CPython's `ast` and resolved against an explicit ordered list of
staged import roots. No repository module, package `__init__.py`, setup script,
or custom import finder is executed.

```sh
excession syntax scan --repo /checkout --rev HEAD --roots src > /tmp/syntax.json
excession syntax resolve --repo /checkout --report /tmp/syntax.json > /tmp/candidates.json
excession syntax native-run --repo /checkout \
  --syntax-report /tmp/syntax.json --resolution-report /tmp/candidates.json \
  --entry src/app/main.py --language python --import-roots src \
  --trusted-local true > /tmp/python-native.json
```

`--import-roots` is **required** for Python and disallowed for other languages.
It accepts 1-16 comma-separated, non-overlapping paths inside the pinned Git
snapshot. Use `.` for the repository root; order matters, following import
precedence. The resolver intentionally ignores ambient `PYTHONPATH`, `sys.path`
entries, editable packages and site-installed distributions. An external path
cannot be declared as Git-authenticated project input. Metadata in
`pyproject.toml`, `setup.cfg` and `setup.py` is retained as configuration
evidence. Declarative setuptools `package-dir` and `find.where` roots are
checked for disagreement when present. Build metadata scripts are **never run**.

## Observable outcomes

The top-level result contains the usual authenticated
`excession.input-declaration.v1`, `stats` and Git/syntax `boundary`, plus
a `resolution` object with schema `excession.python-resolution.v1`.
`resolution.report_id` hashes its canonical contents excluding the ID,
and its Git commit and ordered roots are explicit.

Each observation records source path, line, import specifier, sorted pinned
target paths, reason, and exactly one status:

- `RESOLVED`: a statically resolvable literal import of in-snapshot files.
  It is **not** a runtime execution guarantee.
- `CANDIDATE`: from-import attributes versus submodules, guarded/deferred
  imports, or literal calls to import helpers. The reported files are possible
  inputs, not guaranteed executed dependencies.
- `UNRESOLVED`: target absent from the explicitly configured in-snapshot
  roots. This can represent stdlib/external packages and is **not** a proof
  of an import error at runtime.
- `UNSUPPORTED`: computed import names, wildcard exports, import machinery
  mutations, dynamic code evaluation, invalid relative import context, or
  parse failures.

The resolver follows imports transitively through tracked Python source,
includes `__init__.py` for traversed regular packages, and permits PEP 420
namespace directories split across import roots. The first regular package
or module in ordered roots wins; namespace portions remain composable. A
shadowed entrypoint or ambiguous overlapping roots are refused.

The adapter has a 512-file declared-input limit and 2,048 observed-site limit;
Rust bounds snapshot bytes/count, toolchain execution, source readback, path
identities, output size and assumptions. The Python interpreter, adapter bytes
and platform are declared. The source snapshot is private and read-only, but
native analysis is **trusted-local and not sandboxed or attested**.

## What remains unsupported

A successful report does **not** mean complete Python runtime dependency
discovery. It excludes arbitrary `sys.path` or `sys.meta_path` changes,
project-specific import loaders, source-generated modules, installed editable
packages, compiled extension modules, dynamically computed module names,
implicit filesystem/environment reads, and actual conditional execution.
`setup.py` and `.pth` execution are not modeled.

A `RESOLVED` static import is not evidence that an execution necessarily
imports that file. `UNRESOLVED` and `UNSUPPORTED` observations must be
reviewed, not discarded. `DECLARED_INPUTS_ONLY` stays unchanged and cannot
authorize skipping tests or transferring behavioral guarantees.

This incremental implementation deliberately reuses the source/evidence
contracts and Rust native discovery introduced by PR #50. It does not add a
second source index, SQLite authority, or custom dependency file format.
