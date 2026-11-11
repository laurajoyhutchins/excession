# Composable views and virtual browsing

**Status:** implemented local CLI / terminal browser. Read-only; no FUSE mount, editor extension, network server, or executable checkout. Source authority remains the pinned Git commit.

A derived repository view has a canonical `excession.repository-view.v1` manifest. The original dependency and security-flow generators are unchanged. A composed view has `kind="composed"` and records a `focus` identity derived from the normalized operator and its input view ID(s). Every output entry retains its original Git source path, blob SHA-1, and regular-file mode.

## Operations

Given two manifests over the **same source commit and root tree**, composition works by `(source_path, blob, mode)` identity, not by comparing display names.

| Operator | Meaning | Result paths |
|---|---|---|
| `union` | Retain all references in both views | Original paths from both; conflicting mappings at one path are rejected |
| `intersection` | Keep left references whose source is represented in right | Left paths and metadata, even when right uses a different virtual path |
| `difference` | Keep left references whose source is absent from right | Left paths and metadata |
| `view-select role` | Keep entries with the exact given role | Original paths |
| `view-select source-prefix` | Keep entries within a source path subtree | Original paths |
| `view-select virtual-prefix` | Keep entries within a virtual path subtree | Original paths |
| `view-mount` | Place one view under a stable namespace | `views/<namespace>/...` |

A view must contain at least one entry. Empty selections and intersections are explicit refusals, not silently presented as complete analysis. The existing 5,000-entry ceiling, strict Git modes, and path safety rules apply. Views with different source revisions cannot be combined, even if some blobs happen to be identical. The implementation does **not** claim graph completeness, executable module equivalence, or inferred security impact.

## Reproducible example

Assume `deps.json` and `security.json` were independently generated and replayed according to [repository views](REPOSITORY_VIEWS.md). `REPO` is their pinned source checkout and `BIN` is the locally built Rust binary.

```bash
BIN=./core/target/debug/excession
REPO=/path/to/pinned/checkout
OUT=/tmp/excession-queries
mkdir -p "$OUT"

# Place distinct layouts into different virtual directories.
"$BIN" view mount --repo "$REPO" --manifest "$OUT/deps.json" --namespace dependencies > "$OUT/mounted-deps.json"
"$BIN" view mount --repo "$REPO" --manifest "$OUT/security.json" --namespace security > "$OUT/mounted-security.json"

# Compose without copying the underlying source files.
"$BIN" view compose --repo "$REPO" --op union \
  --left "$OUT/mounted-deps.json" --right "$OUT/mounted-security.json" > "$OUT/composed.json"
"$BIN" view replay compose --repo "$REPO" --op union \
  --left "$OUT/mounted-deps.json" --right "$OUT/mounted-security.json" --output "$OUT/composed.json"

# Drill down into relationships, or read an exact Git blob on demand.
"$BIN" view ls --repo "$REPO" --manifest "$OUT/composed.json" --path views
"$BIN" view ls --repo "$REPO" --manifest "$OUT/composed.json" --path views/security
"$BIN" view stat --repo "$REPO" --manifest "$OUT/composed.json" --path 'views/security/00-findings/…'
"$BIN" view cat --repo "$REPO" --manifest "$OUT/composed.json" --path 'views/security/00-findings/…'

# A terminal REPL using the same Rust verification for each operation.
"$BIN" browse --repo "$REPO" --manifest "$OUT/composed.json"
```

The ellipsis paths are placeholders: obtain real paths from `view-ls`, not from this document. Source bytes are streamed from `git cat-file blob` and checked against their recorded blob IDs. The Python browser escapes terminal control bytes and caps display length. It runs no repository code and uses no shell to invoke the Rust commands.

The browser supports `ls [directory]`, `cd directory`, `pwd`, `stat file`, `show file`, and `quit`. It can also read commands from standard input for scripting. `excession view cat` is the binary-safe API; `show` is a bounded textual inspection convenience.

## Verification boundary

- Each input manifest is parsed, hashed, and authenticated against source Git objects.
- Composition binds its output identity to its input manifest IDs, operation, and optional selector or namespace. Local paths to input manifest files are **not** part of the output identity.
- `view replay compose`, `view replay select`, and `view replay mount` recompute the complete result from the exact input manifests and compare it to the saved output.
- Directory listing, metadata retrieval, and blob reads reauthenticate the view against the pinned source before returning data. There are no writable projections or synthesized imports.
- When the original manifests derive from SQLite dependency or scanner relations, **also** execute `view replay deps` or `view replay security` against their original databases, and independently audit scanner ingestion. Replaying a composition alone cannot establish that those upstream analyses were genuine.
- A manifest is not a signed attestation. An attacker who can replace the source repository, SQLite catalog, and all manifests requires a separate trust root. The current local audit does not authenticate a remote scanning environment.

## Future interfaces

The same Rust `list`, `describe`, and `open` operations can back an editor's read-only virtual file provider. No UI protocol is standardized yet. Writable views and executable relocation require separate explicit source-mapping and native-verification contracts.
