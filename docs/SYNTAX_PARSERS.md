# Multi-language syntax observations

Excession delegates syntax parsing to an existing parser distribution instead of
creating another set of handwritten grammars. The optional adapter uses
[tree-sitter-language-pack](https://github.com/xberg-io/tree-sitter-language-pack),
which exposes precompiled Tree-sitter grammars and conservative language-aware
extraction. The core Rust crate still has **no third-party Cargo dependencies**.

This is separate from the existing CPython AST / TypeScript module-dependency
index, and from the Linux C/Kbuild/Kconfig experiment. No language-specific
dependency, import resolution, type-checking, name binding, or executable behavior
is inferred from this generic syntax scan. Missing facts are not negative evidence.

## Install parsers explicitly

The parser package is optional and must be installed into the Python environment
used by the Excession executable:

```sh
python3 -m pip install tree-sitter-language-pack==1.25.0
python3 -c 'from tree_sitter_language_pack import prefetch; prefetch(["rust","go","c","java","python","javascript","typescript"])'
```

Prefetch happens in a **trusted setup step**, before analysis. Scans never
download grammars. For a configured but uncached grammar, Excession records
`GRAMMAR_NOT_INSTALLED` rather than attempting network access or claiming a
successful parse. The report includes the installed package version; a later
audit requires the same parser version and availability to regenerate it.

## Scan and replay

```sh
excession syntax scan --repo /path/to/clean/checkout \\
  --rev HEAD --roots src,tests > /tmp/syntax.json

excession syntax audit --repo /path/to/clean/checkout \\
  --report /tmp/syntax.json
```

The scan reads **pinned Git blob objects**, not mutable working-tree files.
The manifest records commit, tree, path, blob, mode, parser name/version, source
language, parser status, bounded structural declarations, syntactic imports,
and parser diagnostics. Its `report_id` is a SHA-256 digest over the canonical
payload. The audit re-reads Git, reruns parsing, and compares the *entire*
canonical report, not just the embedded digest.

Limits: 500 eligible regular files, 512 KiB per blob, 16 MiB total source bytes,
2,048 observations per file, 64 nested structural levels, and 32 MiB report
audit size. Narrow `--roots` when a large repository exceeds these limits.
Non-source extensions and vendored / node_modules content are excluded, not
reported as examined. Non-UTF-8 sources are refused.

The current initial extension selection covers Python, JS/TS, Rust, Go, C/C++,
Java, Kotlin, C#, Ruby, PHP, Swift, Scala, shell, Elixir, JSON, YAML, TOML, and
SQL. Actual parser availability and extraction depth differ by language.

Each included file has an explicit status:

| Status | Interpretation |
|---|---|
| `PARSED` | Grammar parsed without reported Tree-sitter error nodes |
| `PARSED_WITH_ERRORS` | Partial parse; do not infer absence of other structure |
| `UNSUPPORTED_LANGUAGE` | No parser mapping for the selected file |
| `GRAMMAR_NOT_INSTALLED` | Grammar not available offline |
| `PARSER_ERROR` | Parser failed; facts discarded |
| `RESOURCE_LIMIT` | Observation budget exceeded; facts discarded |

Even a `PARSED` file does not imply complete import or symbol extraction:
Tree-sitter supplies **syntax**, not the language's build graph, type checker,
or runtime. The package may extract fewer categories for certain languages.
In particular, a syntactic import target is not automatically an Excession
dependency edge. Those edges require language-specific, independently validated
resolvers before they can be inserted into the SQL dependency relation.

## Safety and reproducibility

The adapter runs trusted, locally installed parser binaries and grammars, but
does not run source under examination. Do not load an unreviewed grammar
artifact on a shared privileged host. The package version alone is not a
cryptographic grammar binary attestation. For an independently reproducible
environment, pin downloaded parser artifacts and their hashes externally.

Run unit tests without optional grammars:

```sh
python3 -m unittest tests.test_tree_sitter_bridge -v
```

The integration smoke test uses an intentionally small clean Git fixture and
requires the optional dependency to be installed and prefetched.
