#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${1:-$(mktemp -d "${TMPDIR:-/tmp}/relgit-rust-reproduce.XXXXXXXX")}" 
mkdir -p "$WORK" "$WORK/results" "$WORK/crossrepo"
WORK="$(cd "$WORK" && pwd)"
echo "Reproducing RelGit Rust in $WORK"
cd "$HERE"
cargo fmt -- --check
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline -q
cargo build --offline -q
for pair in 'experimental-base:h11-base' 'agent-A-headers:agent-A-headers' 'agent-B-reader:agent-B-reader' 'integration:integration'; do
    branch=${pair%%:*}; dir=${pair#*:}
    git -c init.defaultBranch=main clone -q --branch "$branch" "$HERE/fixtures/h11.bundle" "$WORK/$dir"
done
export PYTHONDONTWRITEBYTECODE=1 PYTEST_DISABLE_PLUGIN_AUTOLOAD=1
(cd "$WORK/agent-A-headers" && python3 "$HERE/adapters/probe_claim.py" headers --output "$WORK/results/agent-A-headers.json")
(cd "$WORK/agent-B-reader" && python3 "$HERE/adapters/probe_claim.py" reader --output "$WORK/results/agent-B-reader.json")
"$HERE/target/debug/relgit" carry "$WORK" "$WORK/results/relgit.sqlite" "$HERE/fixtures/reference_report.json" > "$WORK/results/rust_carry.json"
(cd "$WORK/integration" && PYTHONPATH="$WORK/integration" python3 "$HERE/adapters/joint_smoke.py")
(cd "$WORK/integration" && PYTHONPATH="$WORK/integration" python3 -m pytest -q -p no:cacheprovider h11/tests/test_headers.py tests/test_reader_contract.py)
for dir in h11-base agent-A-headers agent-B-reader integration; do
    git -C "$WORK/$dir" fsck --full --no-reflogs --no-progress
    test -z "$(git -C "$WORK/$dir" status --porcelain --untracked-files=all)"
done
python3 - "$WORK/results/relgit.sqlite" <<'PY'
import sys,sqlite3
c=sqlite3.connect(sys.argv[1]);assert c.execute('pragma foreign_key_check').fetchall()==[]
print('Four Git trees and SQLite foreign keys: PASS')
PY
for repo in click h11 postcss requests; do
    git -c init.defaultBranch=main clone -q "$HERE/fixtures/${repo}-crossrepo.bundle" "$WORK/crossrepo/$repo"
done
python3 - "$HERE/fixtures/crossrepo_revisions.json" "$HERE/target/debug/relgit" "$WORK" <<'PY'
import json,sys,subprocess
revisions=json.load(open(sys.argv[1]));exe=sys.argv[2];work=sys.argv[3]
for repo,sha in revisions:
    roots=repo+',tests'  # source and separately authored tests
    subprocess.run([exe,'index',work+'/results/crossrepo.sqlite',repo,work+'/crossrepo/'+repo,sha,roots],check=True,stdout=subprocess.DEVNULL)
print(f'Indexed {len(revisions)} cross-repository revisions in Rust')
PY
python3 "$HERE/scripts/check_parity.py" "$HERE/fixtures/expected_crossrepo_fingerprints.json" "$WORK/results/crossrepo.sqlite"
python3 - "$HERE/fixtures/reference_report.json" "$WORK/results/rust_carry.json" <<'PY'
import json,sys
expected=json.load(open(sys.argv[1]));actual=json.load(open(sys.argv[2]))
for k in ('guard_sha256','revision','merge_tree','outcome','carried_tests','new_joint_tests_required','ordinary_git_merge'):
    assert expected[k]==actual[k],f'Rust v9 mismatch: {k}'
print('Exact Rust/Python v9 bounded carry: PASS')
PY
echo "PASS: full Rust differential reproduction and bounded h11 verification"
