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
    --destination core/src/generated_module_name.rs \
    --symbols module_name,module_name_with_import_base \
    --ast-grep "$AST_GREP" "$@"
}
run_recipe > "$TMP/preview.json"
jq -e '.status == "UNVERIFIED_CODEGEN_PREVIEW" and .worktree == null
  and (.linked_functions == ["module_name_with_import_base"])
  and (.additional_helper_functions == ["module_name"])
  and (.edits | length) == 4
  and (.plan_id | length) == 64' "$TMP/preview.json"
test ! -e "$OUT"
# A new worktree must never be nested in the historical checkout.
if run_recipe --output "$OLD/core/unsafe-nested-worktree" > "$TMP/rejected.json" 2>&1; then
  echo "Refactor should refuse a nested output worktree" >&2
  exit 1
fi
test ! -e "$OLD/core/unsafe-nested-worktree"
run_recipe --output "$OUT" > "$TMP/applied.json"
jq -e '.status == "APPLIED_UNVERIFIED"
  and .functions == ["module_name", "module_name_with_import_base"]' "$TMP/applied.json"
jq -e --slurpfile preview "$TMP/preview.json" '
  .plan_id == $preview[0].plan_id and .edits == $preview[0].edits
' "$TMP/applied.json"
grep -Fq 'pub fn module_name(' "$OUT/core/src/generated_module_name.rs"
grep -Fq 'pub fn module_name_with_import_base(' "$OUT/core/src/generated_module_name.rs"
grep -Fq '/// Convert a Git-rooted module identity' "$OUT/core/src/generated_module_name.rs"
grep -Fq 'pub use crate::generated_module_name::{module_name, module_name_with_import_base};' "$OUT/core/src/semantic.rs"
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
echo 'PASS: current evidence-bound native refactor -> committed candidate -> agent context/verify with receipts and unresolved obligations'

# Native symbol-aware code actions use the original historical Git checkout,
# but every edit remains bound to the audited SCIP candidate and source blobs.
run_assist() {
  "$BIN" refactor rust-assist --repo "$OLD" --rev "$OLD_REV" \
    --index "$TMP/index.scip.json" --report "$TMP/scip-report.json" \
    --source-prefix core --candidate "$ID" --source core/src/semantic.rs \
    --start-line 15 --start-character 0 --end-line 74 --end-character 0 \
    --rust-analyzer "$(rustup which --toolchain 1.99.1 rust-analyzer)" "$@"
}
run_assist > "$TMP/assist-actions.json"
jq -e '.status == "AVAILABLE_UNVERIFIED_ACTIONS" and
  any(.actions[]; .title == "Extract Module" and (.kind | startswith("refactor")))' \
  "$TMP/assist-actions.json"
run_assist --action "Extract Module" > "$TMP/assist-preview.json"
jq -e '.status == "UNVERIFIED_CODE_ACTION_PREVIEW"
  and (.edits | length) > 0 and (.plan_id | length) == 64' \
  "$TMP/assist-preview.json"
ASSIST_OUT="$TMP/assisted-candidate"
test ! -e "$ASSIST_OUT"
run_assist --action "Extract Module" --output "$ASSIST_OUT" > "$TMP/assist-applied.json"
jq -e --slurpfile preview "$TMP/assist-preview.json" '
  .status == "APPLIED_UNVERIFIED"
  and .plan_id == $preview[0].plan_id
  and .edits == $preview[0].edits
' "$TMP/assist-applied.json"
test "$(git -C "$OLD" rev-parse HEAD)" = "$OLD_REV"
test -z "$(git -C "$OLD" status --porcelain --untracked-files=all)"
# Commit only the generated source in a detached, isolated candidate.
# This identity is necessary: repair edit ranges must refer to exact Git
# blobs, not unstaged or mutable working-tree bytes.
git -C "$ASSIST_OUT" add core/src/semantic.rs
git -C "$ASSIST_OUT" -c user.name="Excession Replay" \
  -c user.email="excession@example.invalid" commit -qm "Native Extract Module candidate"
test -z "$(git -C "$ASSIST_OUT" status --porcelain --untracked-files=all)"
# Rustc's exact JSON diagnostics are an independent observation. Preserve
# them as input evidence; Excession itself never executes a build script.
if cargo +1.99.1 check --locked --offline --message-format=json \
  --manifest-path "$ASSIST_OUT/core/Cargo.toml" \
  > "$TMP/assist-compiler.jsonl" 2> "$TMP/assist-build.log"; then
  echo "Unexpected successful compilation: update the acceptance oracle" >&2
  exit 1
fi
jq -se 'any(.[]; .reason == "compiler-message"
  and .message.level == "error"
  and .message.code.code == "E0432")' "$TMP/assist-compiler.jsonl"
jq -e '.status == "APPLIED_UNVERIFIED"
  and (.verification | startswith("UNRESOLVED:"))' "$TMP/assist-applied.json"
run_repair() {
  "$BIN" refactor rust-repair --repo "$ASSIST_OUT" --base "$OLD_REV" \
    --source-repo "$OLD" --plan "$TMP/assist-applied.json" \
    --index "$TMP/index.scip.json" --report "$TMP/scip-report.json" \
    --diagnostics "$TMP/assist-compiler.jsonl" \
    --source-prefix core \
    --rust-analyzer "$(rustup which --toolchain 1.99.1 rust-analyzer)" "$@"
}
run_repair > "$TMP/repair-preview.json"
jq -e '
  .status == "REPAIR_PREVIEW_UNVERIFIED"
  and .repair_kind == "byte-conservative-extract-module-api"
  and .parent_helpers == ["module_name"]
  and .action_title == "Preserve public path of module_name_with_import_base"
  and (.edits | length) == 2
  and (.plan_id | length) == 64
' "$TMP/repair-preview.json"

REPAIR_OUT="$TMP/repaired-candidate"
test ! -e "$REPAIR_OUT"
run_repair --output "$REPAIR_OUT" > "$TMP/repair-applied.json"
jq -e --slurpfile preview "$TMP/repair-preview.json" '
  .status == "REPAIR_APPLIED_UNVERIFIED"
  and .plan_id == $preview[0].plan_id
  and .edits == $preview[0].edits
  and (.verification | startswith("OPEN_OBLIGATIONS:"))
' "$TMP/repair-applied.json"
test "$(git -C "$REPAIR_OUT" rev-parse HEAD)" = "$(git -C "$ASSIST_OUT" rev-parse HEAD)"
test -z "$(git -C "$ASSIST_OUT" status --porcelain --untracked-files=all)"
grep -Fq 'pub use self::modname::module_name_with_import_base;' "$REPAIR_OUT/core/src/semantic.rs"
grep -Fq 'use super::module_name;' "$REPAIR_OUT/core/src/semantic.rs"

# Independent compilation and regression tests on the generated worktree.
cargo +1.99.1 fmt --manifest-path "$REPAIR_OUT/core/Cargo.toml"
cargo +1.99.1 fmt --manifest-path "$REPAIR_OUT/core/Cargo.toml" -- --check
cargo +1.99.1 check --locked --offline --manifest-path "$REPAIR_OUT/core/Cargo.toml"
cargo +1.99.1 test --locked --offline --manifest-path "$REPAIR_OUT/core/Cargo.toml" --lib \
  semantic::module_name_tests::semantic_module_names_preserve_import_root_and_test_flags
git -C "$REPAIR_OUT" diff --check

# Public API differential: the same external consumer must compile and produce
# identical behavior against the pre-refactor library and the repaired one.
mkdir -p "$TMP/public-api-client/src"
cat > "$TMP/public-api-client/Cargo.toml" <<EOT
[package]
name = "excession-refactor-public-api-client"
version = "0.1.0"
edition = "2024"
[dependencies]
excession = { path = "$OLD/core" }
EOT
cat > "$TMP/public-api-client/src/main.rs" <<'RS'
fn main() {
    let roots = &["scripts/gdb", "tests"];
    for path in ["scripts/gdb/sample.py", "tests/test_demo.py", "elsewhere/none.py"] {
        let import_base = Some("scripts/gdb");
        let semantic = excession::semantic::module_name_with_import_base(
            path, roots, import_base
        );
        let index = excession::index::module_name_with_import_base(
            path, roots, import_base
        );
        assert_eq!(semantic, index);
        println!("{path}: {semantic:?}");
    }
}
RS
cargo +1.99.1 run --offline --quiet \
  --manifest-path "$TMP/public-api-client/Cargo.toml" > "$TMP/public-api-before.txt"
sed -i "s|$OLD/core|$REPAIR_OUT/core|" "$TMP/public-api-client/Cargo.toml"
cargo +1.99.1 run --offline --quiet \
  --manifest-path "$TMP/public-api-client/Cargo.toml" > "$TMP/public-api-after.txt"
cmp "$TMP/public-api-before.txt" "$TMP/public-api-after.txt"

# Fresh SCIP must bind to the exact new committed tree, not the old cached
# evidence. No trusted inference is made from the stale original index.
git -C "$REPAIR_OUT" add core/src/semantic.rs
git -C "$REPAIR_OUT" -c user.name="Excession Replay" \
  -c user.email="excession@example.invalid" commit -qm "API-preserving module extraction"
PRESERVED_REV="$(git -C "$REPAIR_OUT" rev-parse HEAD)"
test "$PRESERVED_REV" != "$OLD_REV"
test -z "$(git -C "$REPAIR_OUT" status --porcelain --untracked-files=all)"
rustup run 1.99.1 rust-analyzer scip "$REPAIR_OUT/core" \
  --output "$TMP/preserved.scip"
"$SCIP" print --json "$TMP/preserved.scip" > "$TMP/preserved.scip.json"
"$BIN" refactor scip-suggest \
  --repo "$REPAIR_OUT" --rev "$PRESERVED_REV" \
  --index "$TMP/preserved.scip.json" --source-prefix core \
  > "$TMP/preserved-scip-report.json"
jq -e --arg rev "$PRESERVED_REV" '
  .schema == "excession.scip-refactor.v1"
  and .source_commit == $rev
  and .indexer.name == "rust-analyzer"
' "$TMP/preserved-scip-report.json"
echo 'PASS: native Extract Module API preserved, compiler + regression + external public API differential + fresh SCIP verified'
