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
cargo +1.99.1 check --locked --offline --manifest-path "$OUT/core/Cargo.toml"
cargo +1.99.1 test --locked --offline --manifest-path "$OUT/core/Cargo.toml" --lib \
  semantic::module_name_tests::semantic_module_names_preserve_import_root_and_test_flags
git -C "$OUT" diff --check
rustup run 1.99.1 rust-analyzer scip "$OUT/core" --output "$TMP/generated.scip"
"$SCIP" print --json "$TMP/generated.scip" > "$TMP/generated.scip.json"
jq -e 'any(.documents[]; (.relative_path // .relativePath) == "src/generated_module_name.rs")' "$TMP/generated.scip.json"
echo 'PASS: historical function extracted, public alias retained, compiled, freshly indexed'
