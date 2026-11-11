# Repository projections: derived layouts over one Git tree

**Status:** working, read-only, bounded v1, including composable queries and virtual browsing. This is not a source move, a compiled workspace, or a security proof.

A conventional checkout chooses one directory layout. Excession derives alternative navigable trees from the Git-backed relation index. Each virtual file is a reference to a **specific commit, source pathname, regular-file mode, and Git blob OID**. Different views of the same source have the same Git source identities; they are not diverging copies of the program.

## Interface

The installed CLI is `excession`; generated views are not separate source checkouts.
The relevant commands are:

```text
excession index --repo REPO --db DB --name NAME [--rev REV] [--roots ROOTS]
excession view deps --db DB --name NAME --rev REV --seed MODULE
excession view replay deps --db DB --name NAME --rev REV --seed MODULE --manifest JSON
excession view security --db DB --repo REPO --run RUN_ID
excession view replay security --db DB --repo REPO --run RUN_ID --manifest JSON
excession view materialize|audit --repo REPO --manifest JSON --output DIR
```

### Dependency view

First index a clean source checkout with the existing parser/indexer. It already recognizes language-native Python/JavaScript module dependencies within selected roots:

```bash
REPO=/path/to/checkout
OUT=/tmp/excession-work
mkdir -p "$OUT"
excession index --repo "$REPO" --db "$OUT/index.sqlite" --name project --rev HEAD --roots package,tests
excession view deps --db "$OUT/index.sqlite" --name project --rev HEAD --seed package.entry > "$OUT/dependencies.json"
excession view materialize --repo "$REPO" --manifest "$OUT/dependencies.json" --output "$OUT/dependencies-view"
excession view replay deps --db "$OUT/index.sqlite" --name project --rev HEAD --seed package.entry --manifest "$OUT/dependencies.json"
excession view audit --repo "$REPO" --manifest "$OUT/dependencies.json" --output "$OUT/dependencies-view"
```

The layout is deterministic: `00-focus`, `10-dependencies/{direct,transitive}`, `20-dependents/{direct,transitive}`, and `30-tests/{dependencies,dependents}`. Graph traversal uses **shortest reachable distance** and terminates on cycles. A module may deliberately appear in multiple groups, each with the original source blob ID. Only indexed, statically resolved dependencies are represented. Missing dynamic imports, external packages, and runtime relationships are not assumed safe or absent.

### Security-flow view

First run and **independently audit** CodeQL or Semgrep ingestion (see `docs/ARCHITECTURE.md`). The scanner report and SQLite file must live outside the source checkout. After import, use the returned `run_id`:

```bash
excession security ingest --repo "$REPO" --report "$OUT/report.sarif" --db "$OUT/findings.sqlite" --engine codeql
excession security audit --repo "$REPO" --report "$OUT/report.sarif" --db "$OUT/findings.sqlite" --engine codeql
excession view security --db "$OUT/findings.sqlite" --repo "$REPO" --run "$RUN_ID" > "$OUT/security.json"
excession view materialize --repo "$REPO" --manifest "$OUT/security.json" --output "$OUT/security-view"
excession view replay security --db "$OUT/findings.sqlite" --repo "$REPO" --run "$RUN_ID" --manifest "$OUT/security.json"
excession view audit --repo "$REPO" --manifest "$OUT/security.json" --output "$OUT/security-view"
```

The view groups `00-findings`, `10-flow-starts`, `20-flow-intermediate`, and `30-flow-ends`; a single-step thread uses `10-flow-single`. Group names indicate **positions reported by the scanner**, not confirmed sources, sinks, authority boundaries, exploitability, or CVEs. Semgrep candidates lacking CodeQL flows appear only in `00-findings`. Each individual thread step is separately addressable through its candidate, flow, thread, and step numbers.

For filtering, composition, namespaced layouts and interactive browsing, see [composable views](COMPOSABLE_VIEWS.md).

## Deterministic manifest

`view-plan-*` emits strict canonical JSON to standard output, with:

```json
{
  "schema": "excession.repository-view.v1",
  "kind": "dependencies",
  "source_commit": "<exact 40-hex Git SHA-1 commit>",
  "source_tree": "<exact 40-hex Git SHA-1 root tree>",
  "focus": "package.entry",
  "entries": [
    {"virtual_path": "views/00-focus/package/entry.py", "source_path": "package/entry.py", "blob": "<Git blob OID>", "mode": "100644", "role": "focus", "detail": "module=package.entry"}
  ],
  "view_id": "<SHA-256 of canonical JSON without view_id>"
}
```

The placeholders above are illustrative, not real object identities. Entries are sorted by virtual path. The manifest contains **no local absolute paths**, allowing identical Git revisions, index relations, and focus queries to produce identical bytes on independent machines.

`view-plan-replay-*` recomputes the complete selection from the indexed SQLite relations, independently authenticates referenced paths against Git, and compares the regenerated manifest with the saved manifest. `view-audit` separately verifies that every materialized file matches the exact Git blob and that no extra files, missing files, or symlinks have appeared.

The source Git tree remains authoritative. A copied file is not an alternative source of truth.

## Materialization policy

- Output must be a **new, absolute directory** outside the source checkout; no replacement, overlay, or in-place mutation.
- Only tracked regular source blobs (`100644`, `100755`) can be referenced. Git symlinks and submodules are refused.
- Materialization reads source bytes from pinned Git objects rather than the mutable working tree. It produces **read-only copies with mode 0444**, not writable symlinks or hard links.
- Manifest file, source commit, source tree, entry paths, Git blob OIDs, file counts and contents are independently checked.
- Virtual paths live under `views/`, contain no `..`, absolute components, empty components, or backslashes, and are unique.
- Hard bounds: **5,000 entries**, **16 MiB per copied blob**, **128 MiB total copied source**, **5 MiB manifest**. The destination is refused if it already exists. A failed copy removes its own partial output.
- **Not executable by default:** moving whole files changes package paths. The projection does not rewrite imports, run tests, preserve runtime module identity, or certify correctness. The older `transform.rs` physical-layout machinery is separate and requires language-native checks.

A view materialization audit proves byte identity and absence of unexpected files at audit time, not semantic completeness, safety of the program, authenticity of a scanner's external execution environment, or that evidence from a previous revision can be carried forward.

## Tests and measured example

The bundled h11 Git history is a real, pinned repository input, not a synthetic file tree. Tests reproduce two projections over the same commit, build an audited security-flow view using a **controlled CodeQL-format test report**, compare independent regenerations, and challenge the system with deleted dependency relations, changed SQLite flow rows, forged manifests, changed file bytes, unexpected files, symlinks, dirty working trees, and destinations inside the source checkout.

All these tests are offline and require no CodeQL or Semgrep installation. They prove the view machinery, **not independent vulnerability discovery**.

## Next stages (not implemented)

1. Generalize the language-native dependency/index adapters beyond the existing Python/JavaScript support, including Go, Rust, and multi-module repositories.
2. Extend the implemented union/intersection/difference/selection query operators with more relationship families and stable human-readable group names for large graphs.
3. Adapt the implemented read-only virtual file operations and terminal browser to native editor providers; reject edits unless unambiguously mapped to original sources.
4. Independently verified *executable* projections with language-specific import rewrites, integration tests, and Git tree comparisons. This must be a separate opt-in mode with explicit unsupported cases.
5. Benchmark whether focus layouts improve real review tasks, not simply whether a view can be generated.
