#!/usr/bin/env bash
set -euo pipefail
script=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/run_luna.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/workspace" "$tmp/attempts" "$tmp/bin"
git -C "$tmp/workspace" init -q
printf 'example\n' > "$tmp/workspace/example.py"
git -C "$tmp/workspace" add .
git -C "$tmp/workspace" -c user.name=Test -c user.email=test@example.invalid commit -qm smoke
candidate=$(git -C "$tmp/workspace" rev-parse HEAD)
printf 'Review this change\n' > "$tmp/prompt.txt"
sha=$(sha256sum "$tmp/prompt.txt" | cut -d' ' -f1)
jq -n --arg h "$candidate" --arg d "$sha" '{schema:"excession.historical-review-assignment.v1", candidate_commit:$h, prompt_sha256:$d, evidence_sha256:null}' > "$tmp/assignment.json"
cat > "$tmp/bin/codex" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ $1 == login ]]; then
  echo "Logged in using ${MOCK_AUTH:-ChatGPT}"
  exit 0
fi
[[ $1 == exec ]] || exit 9
[[ " $* " == *' --model gpt-6-luna '* ]] || exit 10
[[ " $* " == *' --sandbox read-only '* ]] || exit 11
[[ " $* " == *' model_reasoning_effort="low" '* ]] || exit 12
[[ -z ${OPENAI_API_KEY:-} ]] || exit 13
cat >/dev/null
printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","text":"{\"decision\":\"BLOCK\",\"reason\":\"behavior changed\"}"}}' '{"type":"turn.completed","usage":{"input_tokens":41,"output_tokens":8}}'
MOCK
chmod +x "$tmp/bin/codex"
cat > "$tmp/bin/excession" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == benchmark && $2 == review-record ]] || exit 15
printf '%s\n' "$*" > "${MOCK_RECORD_FILE:?}"
MOCK
chmod +x "$tmp/bin/excession"
export CODEX_BIN="$tmp/bin/codex" EXCESSION_BIN="$tmp/bin/excession" MOCK_RECORD_FILE="$tmp/recorded.args"
export OPENAI_API_KEY='unused-api-key-must-never-be-passed-to-codex'
export MOCK_AUTH='API key'
if bash "$script" "$tmp/assignment.json" "$tmp/attempts/refused.json"; then
  echo 'FAIL: accepted API-key authentication' >&2; exit 1
fi
[[ ! -e $tmp/attempts/refused.codex.jsonl ]] || exit 1
unset MOCK_AUTH
bash "$script" "$tmp/assignment.json" "$tmp/attempts/review.json"
[[ -s $tmp/recorded.args ]]
grep -q -- '--decision BLOCK' "$tmp/recorded.args"
grep -q -- '--input-tokens 41 --output-tokens 8' "$tmp/recorded.args"
[[ -s $tmp/attempts/review.codex.jsonl ]]
echo 'PASS: Luna pinning, API-key refusal, JSONL decision and usage forwarding'
