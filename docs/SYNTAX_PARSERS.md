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
mise run syntax-setup       # fast default: Rust, Go, C, Java, Python, JS/TS
mise run syntax-setup-all   # opt-in: also C++, Kotlin, C#, Ruby, PHP,
                            # Swift, Scala, shell, Elixir, JSON/YAML/TOML/SQL
# Without mise, install requirements/syntax.txt and prefetch grammars with
# the tree-sitter-language-pack Python API in a trusted setup step.
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
Prefetching a grammar does not install its language compiler or establish
compiler-level dependency resolution. Optional compiler environments are
listed in [development setup](DEVELOPMENT.md#other-supported-languages).

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

## Fast preflight before expensive source replay

Run a **parser-free** structural and digest check on saved reports first:

```sh
excession syntax preflight --report /tmp/syntax.json
excession syntax resolve-preflight --report /tmp/candidates.json
```

Preflight validates canonical SHA-256 payload digests, explicit schema/status
values, Git path/OID *formats*, bounded nesting and line offsets, permitted
record fields, source identities, and count limits. It never starts a parser,
opens a Git repository or executes source code. Use it at the point a report
is received, before sending work to a compiler, remote worker, or reviewer.

A preflight result is **`SHAPE_AND_DIGEST_ONLY`**, not proof that source blobs
really exist or that dependencies resolve. Anyone can edit a report and
recompute an unkeyed digest. `syntax audit` and `syntax resolve-audit` still
replay authenticated Git objects and parser results; both now reject malformed
reports **before** replay begins.

Local parser-boundary unit tests can run without grammars:

```sh
python3 -m unittest discover -s tests -p test_tree_sitter_bridge.py -v
cargo test --offline --manifest-path core/Cargo.toml preflight::tests
```

`tests/test_preflight_cli.py` exercises the command surface without
Tree-sitter installed, including accepted-but-untrusted self-consistent
forgeries. Full integration tests remain separate.

## Source-bound dependency candidates

The optional resolver maps syntax imports to tracked files using conservative,
language-specific rules. The result is a separate **candidate** report. It is
not installed into the authoritative SQL dependency index and cannot be used
to justify skipping tests.

```sh
excession syntax resolve --repo /path/to/checkout \
  --report /tmp/syntax.json > /tmp/candidates.json
excession syntax resolve-audit --repo /path/to/checkout \
  --syntax-report /tmp/syntax.json --report /tmp/candidates.json
```

`resolve` first independently replays the entire syntax scan and verifies all
source Git blobs. The resulting `excession.syntax-dependencies.v1` manifest
contains Git-bound `candidate_edges`, `unresolved_imports`, `skipped_sources`,
exact `go.mod` metadata identities, and a canonical `resolution_id`.
`resolve-audit` re-parses and re-resolves both reports and rejects any changed
observation. Missing imports are never treated as evidence of no dependency.

**Supported candidate rules:**

- **JS/TS:** explicitly relative imports mapped to exactly one tracked file,
  using listed extension and `index` candidates; ambiguities and bare imports
  remain unresolved. Does not evaluate `tsconfig` aliases, `package.json`,
  `exports`, bundler settings or transpiler remapping.
- **Python:** dotted and relative module strings mapped to exactly one tracked
  `.py` or `__init__.py` source. Does not evaluate imported members,
  namespace packages, sys.path changes or dynamic imports.
- **Go:** only same-module package imports, using the pinned, authenticated
  nearest `go.mod`. All tracked non-test `.go` package files are candidates;
  build constraints, workspaces, vendoring, generated files and CGO are not
  evaluated.
- **Rust:** no handwritten import or module resolver. Tree-sitter retains
  syntax observations; resolved Rust semantics come from rust-analyzer SCIP.

A candidate edge means only that a **plausible tracked path** matches a parsed
import. Native compiler resolution and complete build graphs remain separate
from this feature. Other languages and missing grammars are explicitly marked
unsupported, not silently assigned empty dependency sets.

## Compiler-declaration boundary, before view assembly

Once an independent language-native resolver has produced an
`excession.input-declaration.v1` file, validate it **before** producing
a repository view. The [compiler-assisted native resolver PR](https://github.com/laurajoyhutchins/excession/pull/22)
is a separate, stacked implementation; this boundary accepts its documented
declaration contract without requiring the resolver to be installed.

```sh
# No Git, Tree-sitter or compiler required for this first check.
excession syntax native-preflight --declaration native.json

# Full Git/parser replay plus a source-scoped comparison:
excession syntax native-compare --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/candidates.json \
  --declaration native.json > /tmp/native-boundary.json

excession syntax native-audit --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/candidates.json \
  --declaration native.json --report /tmp/native-boundary.json
```

The fast preflight accepts only Go, Rust, JavaScript and TypeScript declarations,
and checks language, source subject, 1–512 unique source/configuration inputs,
the native-v1 discovery identifier, toolchain version/platform, and a lowercase
64-hex toolchain fingerprint. It is **not compiler attestation**; the byte-hash
and version are unverified self-reported metadata.

The full comparison requires clean pinned Git, validates every declared file
against the exact commit and its Git blob OID, checks that the subject was
parsed in the selected syntax scan, and **independently regenerates** the entire
syntax candidate report. It then compares only candidate edges whose *source*
belongs to the declared native input set and matches its language.

Results keep separate sets of:
- `matching_edges`: candidate targets also declared as native source inputs
- `static_only_edges`: candidate targets outside that native source scope
- `native_only_sources`: declared source files not explained as candidate targets
- `unscanned_sources` and `unresolved_imports`: observed coverage limitations
- `pinned_inputs`: all declared source/configuration files with exact Git blob IDs

`REVIEW_STATIC_NATIVE_SCOPE_DIFFERENCE` is **not proof that a compiler missed
a dependency**. Build tags, conditional compilation, compiler-owned libraries,
untracked runtime effects, static over-approximation, and an intentionally
narrow syntax scope can all explain differences. Even
`NO_STATIC_NATIVE_DISAGREEMENT_OBSERVED` is not a completeness guarantee.

The source declaration format does not independently pin a compiler run to
a commit. The comparison binds its paths to the caller-selected syntax commit,
but **cannot prove a native compiler produced them for that commit**.
Do not treat this report as independent worker attestation, a verified
behavioral guarantee, or permission to skip tests. Any compiler-backed
promotion needs authenticated runtime evidence in a later contract.

## Execute native discovery and compare immediately

`native-run` is the integrated, opt-in path. Rust stages bounded Git
source blobs and invokes the existing language-native adapters; the produced
`excession.input-declaration.v1` is passed **in memory** to the boundary
checker, with no intermediate user-editable declaration file.

```sh
excession syntax native-run --repo "$REPO" \
  --syntax-report /tmp/syntax.json \
  --resolution-report /tmp/candidates.json \
  --entry src/go/main.go --language go \
  --trusted-local true > /tmp/native-run.json
```

The command accepts `go`, `javascript`, and `typescript`.
Rust's native-run path was removed in favor of rust-analyzer SCIP.
It first rejects malformed syntax/resolution reports, mismatched revisions,
out-of-scope subjects, dirty checkouts, and incorrect Git trees. Only then
does it reconstruct the pinned snapshot and execute the compiler-backed
resolver. The returned JSON carries `declaration`, `stats` and `boundary`,
which includes Git blob identities and any scope differences. The optional
Tree-sitter grammars must be installed ahead of time for full replay.

The reusable Rust authority and narrow Python language adapters originated
from the ongoing [Rust-native discovery PR #32](https://github.com/laurajoyhutchins/excession/pull/32).
This command does not import the remainder of that PR's stacked historical
transport and evidence machinery.

**Trust boundary:** `--trusted-local true` is mandatory because compiler
parsers operate on untrusted source and this is not process isolation.
The result says `TRUSTED_LOCAL_NOT_ATTESTED`: it records a locally observed
compiler invocation and toolchain metadata, **not** an independently attested
remote execution or complete runtime dependency set. Its static-versus-native
comparison is still a scoped hypothesis, never permission to skip tests.

To audit its stable boundary separately, extract the `declaration` and
`boundary` members into individual JSON files and use `native-audit`.
The `stats.elapsed_ns` timing is deliberately not a reproducible identity.

## Safety and reproducibility

The adapter runs trusted, locally installed parser binaries and grammars, but
does not run source under examination. Do not load an unreviewed grammar
artifact on a shared privileged host. The package version records the parser
distribution, not independently authenticated grammar source code. Source
authentication still comes from Excession's Git-bound replay and audit;
do not mistake a successful parse for evidence completeness.

Run unit tests without optional grammars:

```sh
python3 -m unittest tests.test_tree_sitter_bridge -v
```

The integration smoke test uses an intentionally small clean Git fixture and
requires the optional dependency to be installed and prefetched.
