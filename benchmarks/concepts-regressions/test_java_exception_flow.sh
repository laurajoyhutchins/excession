#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
javac -source 8 -target 8 -Xlint:-options -d "$tmp" "$root/JavaExceptionFlow.java"
for phase in A B C; do
  (cd "$root/fixtures/exception-flow/$phase" && java -cp "$tmp" JavaExceptionFlow Parser.java) > "$tmp/$phase.jsonl"
done
jq -es '([.[] | select(.kind=="SOURCE")] | length)==1 and
  ([.[] | select(.method=="Parser#parse()") | select(.guaranteed_throw and .direct_throw_count==0)] | length)==1 and
  ([.[] | select(.method=="Parser#parseNext()") | select(.guaranteed_throw)] | length)==1 and
  ([.[] | select(.method=="Parser#ordinary()") | select(.guaranteed_throw)] | length)==0' "$tmp/A.jsonl" >/dev/null
jq -es '([.[] | select(.method=="Parser#parse()" or .method=="Parser#parseNext()") |
  select(.guaranteed_throw)] | length)==0' "$tmp/B.jsonl" >/dev/null
jq -es '([.[] | select(.method=="Parser#parse()" or .method=="Parser#parseNext()") |
  select(.guaranteed_throw and .direct_throw_count==0)] | length)==2 and
  ([.[] | select(.method=="Parser#ordinary()") | select(.guaranteed_throw)] | length)==0' "$tmp/C.jsonl" >/dev/null
echo "PASS: JDK compiler-AST A guarantee, B loss, C recovery, unchanged negative control"
