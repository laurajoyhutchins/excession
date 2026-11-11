# Python importable names versus indexed source identities

The source index deliberately identifies a Python module by its **tracked Git
path**. For example, `tree/build/lib/acme/consumer.py` is indexed as
`tree.build.lib.acme.consumer`. That name is not necessarily what Python
source imports: `from acme import tools` is valid when the containing
`lib/` directory is on `sys.path`.

The CPython AST adapter now separates those identities:

1. Preserve the existing path-based module name in every SQLite
   `module`, `dependency`, and output record. No table rewrite or change to
   view identities is required.
2. Identify **regular Python packages** using tracked `__init__.py` modules
   inside the selected index scope. The topmost observed package ancestor
   anchors the importable name, regardless of the literal directory name
   preceding it.
3. Resolve absolute and relative static imports using these importable
   aliases, then translate every edge back to the existing indexed module
   name. Reject collisions where two indexed paths claim the same importable
   name rather than silently choosing one.
4. When no package ancestor is observed, preserve the indexed path name.
   Do not invent an environment search path from arbitrary directory names.

For example, when `tree/build/lib/acme/__init__.py` and
`tree/build/lib/acme/tools.py` are in scope, `acme.tools` maps to
`tree.build.lib.acme.tools`. This is not a special case for `src/`, a
project name, or a repository fixture. An observed outer
`workspace/__init__.py` keeps `workspace` as part of the package name.

## Deliberate limits

A tracked `__init__.py` establishes a regular package boundary, **not**
the ambient Python interpreter's import search path. This is a conservative
source-level candidate interpretation. Namespace packages without markers,
editable installs, `PYTHONPATH`, custom finders/loaders, wheel metadata,
import-time side effects, dynamic imports, optional dependencies, runtime
conditionals and environment-specific paths require separate native or
configuration-bound evidence. The adapter cannot certify every runtime
dependency and an absence of an edge is never a safe-test-skipping decision.

If a user-selected scope omits a parent `__init__.py` from the index, it
may be impossible to infer the correct name. Broader, configuration-aware
dependency resolution should use the independently developed native
toolchain path, not additional assumptions baked into this parser.

The Python-specific tests in `tests/test_python_import_names.py` pin
independent expectations for arbitrary-depth regular packages, nested
namespace children under a regular package, an observed outer package,
ambiguous duplicate package names, and unmarked directories that must not
be guessed as import roots.
