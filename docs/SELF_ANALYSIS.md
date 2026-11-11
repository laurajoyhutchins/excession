# Excession examines Excession

Excession's Rust semantic evidence comes from **rust-analyzer's SCIP index**.
Excession does not implement another Rust import/name resolver or parse Cargo
depfiles. The existing Rust SCIP consumer validates the indexed documents
against Git and checks every emitted reference span. The optional Tree-sitter
scan remains a *syntax-only* observation path for all supported languages.

## Advisory CI

[Self-analysis workflow](../.github/workflows/self-analysis.yml) executes on a
clean detached checkout at the workflow commit. It installs the matching
rust-analyzer and rust-src Rust 1.99.1 components, plus the official pinned
SCIP CLI v0.10.0. The workflow:

1. Builds Excession and runs adversarial tests.
2. Runs `rust-analyzer scip` on the `core/` Cargo project in the pinned checkout,
   writing binary SCIP evidence outside source control.
3. Decodes the protobuf with the **official** `scip print --json`.
4. Runs `excession refactor scip-suggest` and `scip-audit` using
   `--source-prefix core`, preserving the upstream index unmodified.
5. Continues to audit generic syntax and conservative *non-Rust* import
   candidates for comparison. Independent Git checks validate source OIDs.

The official SCIP indexer's `project_root` must match the Git project's
`core/` directory. Index document paths are resolved against this root;
incorrect roots, escaping paths, untracked files, incorrect Git blobs and
invalid ranges fail the audit. A SCIP reference is a semantic reference,
**not automatically** a direct import, call, behavioral dependency, or
complete set of runtime inputs.

Evidence artifacts (uploaded even on failure):

| File | Meaning |
| --- | --- |
| `index.scip` | Upstream rust-analyzer protobuf index |
| `index.scip.json` | Official decoded SCIP JSON, unmodified |
| `scip-report.json` | Git-bound symbol/reference and advisory refactor hypotheses |
| `scip-audit.json` | Reconstructed SCIP report replay receipt |
| `syntax.json`, `syntax-audit.json` | General source-bound syntax and parser availability |
| `dependencies.json`, `dependencies-audit.json` | Non-Rust conservative path candidates and unknowns |
| `summary.json`, `summary.md` | Coverage, uncertain cases, timings, source changes |

## Reproduce

Prerequisites: Rust 1.99.1 with rust-analyzer and rust-src, Python 3.13,
the existing Tree-sitter parser requirements, the official SCIP CLI v0.10.0,
Git, and system SQLite/OpenSSL.

```sh
rustup component add --toolchain 1.99.1 rust-analyzer rust-src
git clone --filter=blob:none --no-checkout https://github.com/scip-code/scip.git /tmp/scip-source
git -C /tmp/scip-source checkout --detach 1c2b6db7e560d5233c944f36e4ac1377cc6963fc
(cd /tmp/scip-source && go build -o /tmp/scip ./cmd/scip)
cargo +1.99.1 build --offline --locked --manifest-path core/Cargo.toml --bin excession
SOURCE="$(mktemp -d)"
OUT="$(mktemp -d)"
git worktree add --detach "$SOURCE" HEAD
export CARGO_TARGET_DIR="$(mktemp -d)"
rustup run 1.99.1 rust-analyzer scip "$SOURCE/core" --output "$OUT/index.scip"
/tmp/scip print --json "$OUT/index.scip" > "$OUT/index.scip.json"
python3 tools/self_analysis.py \
  --repo "$SOURCE" --rev HEAD --base-rev HEAD^ \
  --bin "$PWD/core/target/debug/excession" \
  --scip-index "$OUT/index.scip.json" --out "$OUT"
git worktree remove "$SOURCE"
```

The upstream rust-analyzer command loads a Cargo workspace and may interact
with configured build scripts, proc macros, or toolchains. Only analyze trusted
projects locally; isolate untrusted builds. Tool versions and raw SCIP bytes
are retained, but a Git-bound SCIP report is **not** an independent attestation
of the indexer's execution or correctness.

The existing Rust/Python test suites remain authoritative. Excession never
uses its observations to skip tests, approve changes, or claim complete
dependency or behavioral closure. The SCIP output and syntax candidates have
different semantics and must not be combined into an invented "recall"
fraction without an independently defined denominator.
