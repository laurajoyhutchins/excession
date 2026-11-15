#!/usr/bin/env bash
# Held-out third-party acceptance at the revision immediately before the
# upstream author's independently recorded unused-import cleanup.
# No modification to the upstream repository, and no synthetic source injected.
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: heldout_improve_upstream.sh EXCESSION_ROOT BASE_REPO CANDIDATE_WORKTREE RECEIPTS" >&2
  exit 2
fi

EXCESSION_ROOT="$1"
BASE_REPO="$2"
CANDIDATE="$3"
RECEIPTS="$4"
SOURCE="packages/core/src/machine/types.ts"
PIN="78c6a78b8a367c2c19398b05d39274ea3bcacadb"
OWNER="piyushrav1/ai-interview-v.01"
UPSTREAM_FIX="dea7976fcf620fc19c9731edef06a3d294f0451c"
mkdir -p "$RECEIPTS"

if [[ "$(git -C "$BASE_REPO" rev-parse HEAD)" != "$PIN" ]]; then
  echo "third-party repository revision differs from pinned upstream base" >&2
  exit 1
fi
if [[ -n "$(git -C "$BASE_REPO" status --porcelain --untracked-files=all)" ]]; then
  echo "third-party source checkout must be clean before native analysis" >&2
  exit 1
fi

PLAN="$RECEIPTS/plan.json"
EDITS="$RECEIPTS/edits.json"
EXCESSION="$EXCESSION_ROOT/core/target/debug/excession"

# Native TypeScript language service sees the exact authenticated Git blob,
# independently of the developer's uncommitted changes or workspace modules.
node "$EXCESSION_ROOT/core/adapters/improve_typescript.cjs" \
  --repo "$BASE_REPO" --rev "$PIN" --source "$SOURCE" \
  --symbol Turn --plan-out "$PLAN" --edits-out "$EDITS" \
  > "$RECEIPTS/native-proposal.json"

jq -e --arg pin "$PIN" --arg path "$SOURCE" '
  .schema == "excession.improvement-plan.v1" and
  .source_commit == $pin and
  .producer == "typescript-native" and
  .ready_to_merge == false and
  (.candidate_count == 1) and
  (.candidates[0].finding.locations[0].path == $path) and
  (.candidates[0].finding.locations[0].symbol == "Turn")
' "$PLAN" > /dev/null
jq -e --arg pin "$PIN" --arg path "$SOURCE" '
  .schema == "excession.improvement-native-edits.v1" and
  .source_commit == $pin and
  .native_producer == "typescript-language-service@5.8.3" and
  (.edits | length) >= 1 and
  all(.edits[]; .path == $path and .replacement == "" and .end_byte > .start_byte)
' "$EDITS" > /dev/null

"$EXCESSION" improve replay --repo "$BASE_REPO" --rev "$PIN" \
  --plan "$PLAN" --edits "$EDITS" > "$RECEIPTS/preview.json"
jq -e '
  .status == "UNVERIFIED_EDIT_PREVIEW" and
  .ready_to_merge == false and
  .automatic_deletion_authorized == false and
  (.obligations | index("NATIVE_PRODUCER_AUTHENTICITY_UNVERIFIED") != null)
' "$RECEIPTS/preview.json" > /dev/null

"$EXCESSION" improve replay --repo "$BASE_REPO" --rev "$PIN" \
  --plan "$PLAN" --edits "$EDITS" --output "$CANDIDATE" \
  > "$RECEIPTS/applied.json"
jq -e '
  .status == "APPLIED_UNVERIFIED" and
  .source_changes == 1 and
  .ready_to_merge == false and
  .automatic_deletion_authorized == false
' "$RECEIPTS/applied.json" > /dev/null

git -C "$CANDIDATE" diff -- "$SOURCE" > "$RECEIPTS/change.diff"
if [[ "$(git -C "$CANDIDATE" diff --name-only)" != "$SOURCE" ]]; then
  echo "native edit unexpectedly touched additional Git source files" >&2
  exit 1
fi

node - "$BASE_REPO/$SOURCE" "$CANDIDATE/$SOURCE" "$RECEIPTS/measurement.json" "$PIN" "$OWNER" "$UPSTREAM_FIX" <<'NODE'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const [oldPath, newPath, reportPath, pin, owner, fix] = process.argv.slice(2);
const oldSource = fs.readFileSync(oldPath, 'utf8');
const newSource = fs.readFileSync(newPath, 'utf8');
assert.equal((oldSource.match(/\bTurn\b/g) || []).length, 1);
assert.equal((newSource.match(/\bTurn\b/g) || []).length, 0);
for (const name of ['Phase', 'CodeSnapshot', 'Interview', 'InterviewState']) {
  assert.ok(newSource.includes(name), `preserved binding disappeared: ${name}`);
}
assert.ok(newSource.includes('from "@testmeman/contracts"'));
const before = Buffer.byteLength(oldSource);
const after = Buffer.byteLength(newSource);
assert.ok(after < before, 'native edit must delete actual bytes');
const receipt = {
  schema: 'excession.heldout-source-reduction.v1',
  upstream_repository: owner,
  source_commit: pin,
  independently_authored_cleanup_commit: fix,
  before_bytes: before,
  after_bytes: after,
  removed_bytes: before - after,
  deleted_import_binding: 'Turn',
  new_source_files: 0,
  changed_files: 1,
  code_compiles_and_tests: 'NOT_YET_RUN',
  ready_to_merge: false,
};
fs.writeFileSync(reportPath, JSON.stringify(receipt, null, 2) + '\n');
NODE

# Two independent installations using the pinned upstream lockfile, so
# workspace links in one checkout cannot silently resolve to the other.
# Ignore lifecycle scripts. No untrusted package hooks run on this worker.
pnpm --dir "$BASE_REPO" install --frozen-lockfile --ignore-scripts \
  > "$RECEIPTS/baseline-dependencies.log" 2>&1
pnpm --dir "$CANDIDATE" install --frozen-lockfile --ignore-scripts \
  > "$RECEIPTS/candidate-dependencies.log" 2>&1

# Use the upstream project's tsconfig and tests. The historical
# pnpm lock resolves TypeScript 5.9.3 despite its declared ^5.6.0 range;
# TS5.9 rejects the pre-existing baseUrl option in BOTH trees. A separately
# installed 5.6.3 compiler, inside that published range and independent of
# Excession's 5.8.3 edit producer, checks both revisions without modifying
# either source tree or the upstream package-lock contract.
: "${HELDOUT_TSC:?require independently pinned TS compiler path}"
test -f "$HELDOUT_TSC"
node "$HELDOUT_TSC" --version > "$RECEIPTS/independent-compiler.txt"
(
  cd "$BASE_REPO"
  node "$HELDOUT_TSC" --noEmit --project packages/core/tsconfig.json
) > "$RECEIPTS/baseline-build.log" 2>&1
(
  cd "$CANDIDATE"
  node "$HELDOUT_TSC" --noEmit --project packages/core/tsconfig.json
) > "$RECEIPTS/candidate-build.log" 2>&1
# Upstream's vitest.config.ts defines test discovery relative to the
# workspace root, not packages/core. Invoke it there with an explicit core
# test filter; pnpm --filter @testmeman/core test would find zero tests.
(
  cd "$BASE_REPO"
  pnpm exec vitest run packages/core/test
) > "$RECEIPTS/baseline-test.log" 2>&1
(
  cd "$CANDIDATE"
  pnpm exec vitest run packages/core/test
) > "$RECEIPTS/candidate-test.log" 2>&1

if [[ "$(git -C "$CANDIDATE" diff --name-only)" != "$SOURCE" ]]; then
  echo "third-party candidate unexpectedly changed tracked source during tests" >&2
  exit 1
fi
if [[ -n "$(git -C "$BASE_REPO" diff --name-only)" ]]; then
  echo "third-party baseline unexpectedly changed tracked source during tests" >&2
  exit 1
fi
node - "$RECEIPTS/measurement.json" <<'NODE'
const fs = require('node:fs');
const path = process.argv[2];
const result = JSON.parse(fs.readFileSync(path, 'utf8'));
result.code_compiles_and_tests = 'BOTH_BASELINE_AND_CANDIDATE_PASSED_UPSTREAM_CORE_CHECKS';
fs.writeFileSync(path, JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result));
NODE
echo "Third-party source reduction OBSERVED_UNVERIFIED; no upstream change or auto-merge."
