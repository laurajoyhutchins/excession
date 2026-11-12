#!/usr/bin/env bash
# Native rust-analyzer => official SCIP => Excession => ast-grep.
set -euo pipefail
BIN="$1"; AST_GREP="$2"; SCIP="$3"; TMP="$4"; REPO="$5"
OLD_REV=19f294ec9ce518349ef3866032b17455c5bdab0a
OLD="$TMP/pre-161-source"; OUT="$TMP/generated-candidate"
mkdir -p "$TMP"
git -C "$REPO" worktree add --detach "$OLD" "$OLD_REV"
rustup run 1.99.1 rust-analyzer scip "$OLD/core" --output "$TMP/index.scip"
"$SCIP" print --json "$TMP/index.scip" > "$TMP/index.scip.json"
"$BIN" refactor scip-suggest --repo "$OLD" --rev "$OLD_REV" \
  --index "$TMP/index.scip.json" --source-prefix core > "$TMP/scip-report.json"
ID="$(jq -r 'first(.candidates[] | select(.kind == "reciprocal_symbol_references") | select(any(.sources[]; .path == "core/src/semantic.rs")) | .candidate_id) // empty' "$TMP/scip-report.json")"
test -n "$ID"
"$BIN" agent context --repo "$OLD" --rev "$OLD_REV" \
  --focus core/src/semantic.rs --task "Extract a shared Rust module" \
  --roots core/src --limit 8 > "$TMP/agent-context.json"
jq -e --arg base "$OLD_REV" \
  '.schema == "excession.agent-context.v1" and .source_commit == $base and .sources[0].path == "core/src/semantic.rs" and .coverage.status == "PARTIAL" and .safe_to_skip_unlisted_tests == false' \
  "$TMP/agent-context.json"
run_recipe() {
  "$BIN" refactor extract-shared-module --repo "$OLD" --rev "$OLD_REV" \
    --index "$TMP/index.scip.json" --report "$TMP/scip-report.json" \
    --source-prefix core --candidate "$ID" --source core/src/semantic.rs \
    --destination core/src/generated_module_name.rs --symbols module_name \
    --ast-grep "$AST_GREP" "$@"
}
run_recipe > "$TMP/preview.json"
jq -e '.status == "UNVERIFIED_CODEGEN_PREVIEW" and .worktree == null' "$TMP/preview.json"
test ! -e "$OUT"
run_recipe --output "$OUT" > "$TMP/applied.json"
jq -e '.status == "APPLIED_UNVERIFIED" and .functions == ["module_name"]' "$TMP/applied.json"
grep -Fq 'pub fn module_name(' "$OUT/core/src/generated_module_name.rs"
grep -Fq 'pub use crate::generated_module_name::{module_name};' "$OUT/core/src/semantic.rs"
grep -Fq 'mod generated_module_name;' "$OUT/core/src/lib.rs"
test "$(git -C "$OLD" rev-parse HEAD)" = "$OLD_REV"
test -z "$(git -C "$OLD" status --porcelain --untracked-files=all)"
cargo +1.99.1 fmt --manifest-path "$OUT/core/Cargo.toml"
cargo +1.99.1 fmt --manifest-path "$OUT/core/Cargo.toml" -- --check
cargo +1.99.1 check --locked --offline --manifest-path "$OUT/core/Cargo.toml" \
  > "$TMP/build.log" 2>&1
cargo +1.99.1 test --locked --offline --manifest-path "$OUT/core/Cargo.toml" --lib \
  semantic::module_name_tests::semantic_module_names_preserve_import_root_and_test_flags \
  > "$TMP/test.log" 2>&1
git -C "$OUT" diff --check
rustup run 1.99.1 rust-analyzer scip "$OUT/core" --output "$TMP/generated.scip"
"$SCIP" print --json "$TMP/generated.scip" > "$TMP/generated.scip.json"
jq -e 'any(.documents[]; (.relative_path // .relativePath) == "src/generated_module_name.rs")' "$TMP/generated.scip.json"
git -C "$OUT" add core/src/semantic.rs core/src/generated_module_name.rs core/src/lib.rs
git -C "$OUT" -c user.name="Excession Replay" \
  -c user.email="excession@example.invalid" commit -qm "Extract pinned shared module"
CANDIDATE="$(git -C "$OUT" rev-parse HEAD)"
TREE="$(git -C "$OUT" rev-parse 'HEAD^{tree}')"
BUILD_LOG_SHA="$(sha256sum "$TMP/build.log" | cut -d' ' -f1)"
TEST_LOG_SHA="$(sha256sum "$TMP/test.log" | cut -d' ' -f1)"
jq -n --arg commit "$CANDIDATE" --arg tree "$TREE" \
  --arg build "$BUILD_LOG_SHA" --arg test "$TEST_LOG_SHA" \
  '{schema:"excession.execution-receipts.v1",receipts:[
    {kind:"BUILD",status:"PASS",source_commit:$commit,source_tree:$tree,
      producer:"refactor-codegen-replay",command:"cargo check --locked --offline",
      exit_code:0,log_sha256:$build},
    {kind:"TEST",status:"PASS",source_commit:$commit,source_tree:$tree,
      producer:"refactor-codegen-replay",command:"cargo test --locked --offline --lib semantic::module_name_tests::semantic_module_names_preserve_import_root_and_test_flags",
      exit_code:0,log_sha256:$test}
  ]}' > "$TMP/agent-receipts.json"
"$BIN" agent verify --repo "$OUT" --base "$OLD_REV" \
  --candidate "$CANDIDATE" --roots core/src --limit 16 \
  --receipts "$TMP/agent-receipts.json" > "$TMP/agent-verification.json"
jq -e --arg base "$OLD_REV" --arg head "$CANDIDATE" --arg tree "$TREE" \
  '.schema == "excession.agent-verification.v1" and .base_commit == $base
   and .candidate_commit == $head and .candidate_tree == $tree
   and .change_count == 3 and (.receipts | length) == 2
   and .replay.source == "RECOMPUTED_FROM_PINNED_GIT"
   and (.obligations | index("RECEIPT_AUTHENTICITY_UNVERIFIED") != null)
   and .status == "OPEN_OBLIGATIONS" and .ready_to_merge == false
   and .safe_to_skip_unlisted_tests == false' \
  "$TMP/agent-verification.json"
echo 'PASS: native refactor -> committed revision -> Rust agent context/verify -> bounded receipts with unresolved obligations'
