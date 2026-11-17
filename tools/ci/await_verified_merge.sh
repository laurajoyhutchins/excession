#!/usr/bin/env bash
# Last gate for trusted-main, native-only improvement candidates.
# Requires GH_TOKEN, PR, BRANCH, HEAD, BASE. No source code is executed.
set -euo pipefail

: "${GH_TOKEN:?GitHub token required}"
: "${PR:?PR number required}"
: "${BRANCH:?Exact bot branch required}"
: "${HEAD:?Exact candidate commit required}"
: "${BASE:?Exact main commit required}"
[[ "$BASE" =~ ^[a-f0-9]{40}$ ]] || exit 2
[[ "$HEAD" =~ ^[a-f0-9]{40}$ ]] || exit 2
[[ "$PR" =~ ^[0-9]+$ ]] || exit 2
[[ "$BRANCH" =~ ^automation/(native-rustfmt|cargo-refresh)-[0-9]+$ ]] || exit 2

# A green status from another revision, a PR event or a skipped workflow
# must never be substituted for an exact-head workflow_dispatch receipt.
VERIFIED=false
for attempt in $(seq 1 72); do
  VERIFY="$(gh run list --workflow verify.yml --branch "$BRANCH" \
    --event workflow_dispatch --limit 30 \
    --json databaseId,headSha,status,conclusion \
    | jq -r --arg sha "$HEAD" '[.[] | select(.headSha == $sha)] | sort_by(.databaseId) | last | if . == null or .status != "completed" then "pending" else .conclusion end')"
  EVIDENCE="$(gh run list --workflow github-evidence.yml --branch "$BRANCH" \
    --event workflow_dispatch --limit 30 \
    --json databaseId,headSha,status,conclusion \
    | jq -r --arg sha "$HEAD" '[.[] | select(.headSha == $sha)] | sort_by(.databaseId) | last | if . == null or .status != "completed" then "pending" else .conclusion end')"
  echo "Exact head $HEAD: Verify=$VERIFY; GitHub evidence=$EVIDENCE"
  if [ "$VERIFY" = success ] && [ "$EVIDENCE" = success ]; then
    VERIFIED=true
    break
  fi
  for result in "$VERIFY" "$EVIDENCE"; do
    if [ "$result" != pending ] && [ "$result" != success ]; then
      echo "REFUSED: independent verification failed, cancelled, skipped or was unsupported."
      exit 1
    fi
  done
  sleep 20
done
test "$VERIFIED" = true || { echo "REFUSED: exact-head CI receipt timeout"; exit 1; }

# Do not rebase after verification: a rebased head has not passed these checks.
REMOTE_BASE="$(git ls-remote origin refs/heads/main | cut -f1)"
test "$REMOTE_BASE" = "$BASE" || { echo "REFUSED: main advanced"; exit 1; }
gh api "repos/$GITHUB_REPOSITORY/pulls/$PR" > "$RUNNER_TEMP/auto-pr.json"
jq -e --arg base "$BASE" --arg head "$HEAD" \
  '.state == "open" and .draft == false and .base.sha == $base and .head.sha == $head' \
  "$RUNNER_TEMP/auto-pr.json"
gh pr merge "$PR" --squash --delete-branch \
  --subject "chore: integrate verified native improvement"
echo "Merged exact-head verified candidate $HEAD (PR #$PR)." >> "$GITHUB_STEP_SUMMARY"
