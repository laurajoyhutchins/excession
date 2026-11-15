#!/usr/bin/env bash
# Held-out acceptance only: the native producer has not seen this application
# in its unit tests. It must discover a real, existing redundant import.
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: heldout_improve_clod_pet.sh EXCESSION_ROOT BASE_REPO CANDIDATE_WORKTREE RECEIPTS" >&2
  exit 2
fi

EXCESSION_ROOT="$1"
BASE_REPO="$2"
CANDIDATE="$3"
RECEIPTS="$4"
SOURCE="app/src/main/pet-manager.ts"
PIN="ba9b87d7434d70ce2569fbdb7420ec2b73a73729"
mkdir -p "$RECEIPTS"

if [[ "$(git -C "$BASE_REPO" rev-parse HEAD)" != "$PIN" ]]; then
  echo "held-out repository revision is not the pinned commit" >&2
  exit 1
fi
if [[ -n "$(git -C "$BASE_REPO" status --porcelain --untracked-files=all)" ]]; then
  echo "held-out base checkout is not clean before native analysis" >&2
  exit 1
fi

PLAN="$RECEIPTS/plan.json"
EDITS="$RECEIPTS/edits.json"
EXCESSION="$EXCESSION_ROOT/core/target/debug/excession"

# Native TypeScript diagnostics run on the authenticated Git bytes. No
# reference to the user's current checkout or uncommitted source is accepted.
node "$EXCESSION_ROOT/core/adapters/improve_typescript.cjs" \
  --repo "$BASE_REPO" --rev "$PIN" --source "$SOURCE" \
  --symbol PetWindowOptions --plan-out "$PLAN" --edits-out "$EDITS" \
  > "$RECEIPTS/native-proposal.json"

jq -e --arg pin "$PIN" --arg path "$SOURCE" '
  .schema == "excession.improvement-plan.v1" and
  .source_commit == $pin and
  .producer == "typescript-native" and
  .ready_to_merge == false and
  (.candidate_count == 1) and
  (.candidates[0].finding.locations[0].path == $path) and
  (.candidates[0].finding.locations[0].symbol == "PetWindowOptions")
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
changed="$(git -C "$CANDIDATE" diff --name-only)"
if [[ "$changed" != "$SOURCE" ]]; then
  echo "held-out native edit changed unexpected Git source files" >&2
  exit 1
fi

node - "$BASE_REPO/$SOURCE" "$CANDIDATE/$SOURCE" "$RECEIPTS/measurement.json" <<'NODE'
const assert = require('node:assert/strict');
const fs = require('node:fs');
const [oldPath, newPath, reportPath] = process.argv.slice(2);
const oldSource = fs.readFileSync(oldPath, 'utf8');
const newSource = fs.readFileSync(newPath, 'utf8');
assert.equal((oldSource.match(/\bPetWindowOptions\b/g) || []).length, 1);
assert.equal((newSource.match(/\bPetWindowOptions\b/g) || []).length, 0);
for (const name of ['WorldStore', 'PetInstance', 'BackendWorldContext', 'standardizeError']) {
  assert.ok(newSource.includes(name), `used import or symbol disappeared: ${name}`);
}
assert.ok(newSource.includes('from "../shared/store"'));
const before = Buffer.byteLength(oldSource);
const after = Buffer.byteLength(newSource);
assert.ok(after < before, 'expected actual source-byte deletion');
const evidence = {
  schema: 'excession.heldout-source-reduction.v1',
  before_bytes: before,
  after_bytes: after,
  removed_bytes: before - after,
  deleted_import_binding: 'PetWindowOptions',
  new_source_files: 0,
  changed_files: 1,
  code_compiles_and_tests: 'NOT_YET_RUN',
  ready_to_merge: false
};
fs.writeFileSync(reportPath, JSON.stringify(evidence, null, 2) + '\n');
NODE

# Install all project dependencies *once* with lifecycle scripts disabled.
# Both checkouts use the exact pinned package-lock and the same package graph.
# This is a disposable CI worker. No dependency installs run on the user host.
npm --prefix "$BASE_REPO/app" ci --ignore-scripts --no-audit --no-fund \
  > "$RECEIPTS/dependency-install.log" 2>&1
ln -s "$BASE_REPO/app/node_modules" "$CANDIDATE/app/node_modules"

# Independent application toolchain: the held-out package pins TypeScript
# 6.x, unlike Excession's 5.8.3 code-fix producer.
(
  cd "$BASE_REPO/app"
  npm run build:ts
) > "$RECEIPTS/baseline-build.log" 2>&1
(
  cd "$CANDIDATE/app"
  npm run build:ts
) > "$RECEIPTS/candidate-build.log" 2>&1

# Run the actual application behavior tests, not merely a syntax check.
(
  cd "$BASE_REPO/app"
  ./node_modules/.bin/jest tests/unit/pet-manager.test.ts --runInBand --no-coverage
) > "$RECEIPTS/baseline-test.log" 2>&1
(
  cd "$CANDIDATE/app"
  ./node_modules/.bin/jest tests/unit/pet-manager.test.ts --runInBand --no-coverage
) > "$RECEIPTS/candidate-test.log" 2>&1

# Validate that no untracked build artifacts were mistaken for tracked edits.
if [[ "$(git -C "$CANDIDATE" diff --name-only)" != "$SOURCE" ]]; then
  echo "application build unexpectedly modified tracked candidate source" >&2
  exit 1
fi
if [[ -n "$(git -C "$BASE_REPO" diff --name-only)" ]]; then
  echo "application baseline build unexpectedly modified tracked source" >&2
  exit 1
fi
node - "$RECEIPTS/measurement.json" <<'NODE'
const fs = require('node:fs');
const file = process.argv[2];
const report = JSON.parse(fs.readFileSync(file, 'utf8'));
report.code_compiles_and_tests = 'BOTH_BASELINE_AND_CANDIDATE_PASSED_TARGETED_CHECKS';
fs.writeFileSync(file, JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report));
NODE
echo "Held-out source reduction accepted as OBSERVED_UNVERIFIED; not a merge authorization."
