# Excession examines Excession

The first self-hosting experiment is deliberately **advisory**. It uses the
actual Excession binary to parse a clean, pinned revision of Excession's own
Rust, adapters, tests, tools, and benchmarks. It replays both reports, then
separately checks every reported source path, mode, and blob OID against Git.

This is a development observation, not a proof of complete dependencies,
correct test selection, behavioral equivalence, or a refactoring verdict.

## CI

The dedicated path-filtered workflow
[Excession self-analysis](../.github/workflows/self-analysis.yml) runs on
applicable PRs, pushes to main, or manual dispatch. Ordinary Rust/Python
verification remains independent and continues to run regardless of these
results. The workflow does not modify source or approve a merge.

It constructs a clean detached Git worktree at the *exact* workflow commit.
All reports go outside that worktree. The workflow preserves these artifacts:

| File | Contents |
| --- | --- |
| syntax.json | Full source-bound syntax observations and parser statuses |
| syntax-audit.json | Re-parse and Git replay receipt |
| dependencies.json | Conservative candidate edges, unresolved imports, skipped sources |
| dependencies-audit.json | Independently regenerated resolution receipt |
| summary.json | Independently Git-checked counts, revision delta, advisory impact hints, measured phase times |
| summary.md | Short human-readable CI job summary |

A missing grammar, partially parsed file, unresolved import, external import,
or unsupported language is **not** evidence of an absent dependency. The report
retains these categories instead of collapsing them to zero. Candidate impact
is a graph traversal over *reported candidates only*; it cannot authorize
skipping any test. We do not compare the candidate graph to the Cargo compiler
closure or claim semantic completeness.

A failure in the source, independent blob, replay, or report consistency checks
fails this dedicated job, but does not turn unknown dependencies into a
negative finding. The driver refuses a dirty checkout and writes evidence
outside the analyzed Git worktree.

## Reproduce locally

With Rust 1.99.1, Python 3.13.5, Git and system SQLite/OpenSSL available:

\`\`\`sh
cargo +1.99.1 build --locked --offline --manifest-path core/Cargo.toml --bin excession
python3 -m pip install tree-sitter-language-pack==1.25.0
python3 -c 'from tree_sitter_language_pack import prefetch; prefetch(["rust", "python", "javascript", "typescript", "go", "c", "java"])'

SOURCE="$(mktemp -d)"
git worktree add --detach "$SOURCE" HEAD
python3 tools/self_analysis.py \
  --repo "$SOURCE" \
  --bin "$PWD/core/target/debug/excession" \
  --out /tmp/excession-self-analysis \
  --rev HEAD \
  --base-rev HEAD^
git worktree remove "$SOURCE"
\`\`\`

The script also accepts \`--roots\` to narrow a scan if the 500-file,
16 MiB aggregate, or 512 KiB per-source syntax limits are exceeded.
This changes the observed scope, so compare only receipts with equivalent
scopes. Source commit and Git tree identity are recorded in every report.

Run the independent driver tests without parsers or Rust:

\`\`\`sh
python3 -m unittest discover -s tests -p 'test_self_analysis.py' -v
\`\`\`

## Success criterion for the next iteration

Use the advisory source and change-impact outputs on a real Excession PR.
Compare their exact changed paths and source relationships with an independent
compiler or repository-navigation baseline. Report true, missing, extra, and
unresolved relationships, plus runtime and bytes of evidence. Only then
evaluate whether a proposed refactor or retrieval workflow saves developer
work. Avoid agent task-completion scores that lack an independently observable
denominator.
