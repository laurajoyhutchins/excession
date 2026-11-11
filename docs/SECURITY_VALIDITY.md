# Validity of CodeQL and Semgrep findings

## What is now independently established

`security ingest` and `security audit` preserve source-bound scanner *reports*;
that is not a statement about detection accuracy. The opt-in local assessor in
`tools/assess_security_findings.py` joins an **already audited** ingestion run
to Git-blob-bound behavioral controls and explicit scanner rule IDs. It deliberately produces a separate
receipt and never changes `security_ingest_candidate` or assigns CVE verdicts.

A controlled Go corpus in `benchmarks/security-validity` demonstrates one
parent-component outside-root file read, a guard that rejects that traversal,
continued benign access through the guard, and a safe constant-path read. The
oracle is the independent Go test code, not the scanner or the source pattern.
It does not establish application reachability, symlink safety, general
path-traversal absence, or exploitability in a deployed service.

## Run the experiment

Use a **trusted** clean checkout of Excession, with the built Rust CLI, Go and
installed Semgrep available. Run the scanner from the checkout root; store the
output and SQLite file outside the repository. The example Semgrep rule is
intentionally broad enough to exercise a potential false positive as well as
an actual outside-root read. It is an evaluation probe, not a recommended
production security rule.

```sh
REPO=$(git rev-parse --show-toplevel)
OUT=$(mktemp -d)
cargo build --locked --manifest-path "$REPO/core/Cargo.toml" --bin excession

semgrep scan --metrics off --config "$REPO/benchmarks/security-validity/semgrep.yml" \
  --json --output "$OUT/semgrep.json" benchmarks/security-validity

"$REPO/core/target/debug/excession" security ingest --repo "$REPO" \
  --report "$OUT/semgrep.json" --db "$OUT/findings.sqlite" --engine semgrep \
  > "$OUT/ingest.json"
"$REPO/core/target/debug/excession" security audit --repo "$REPO" \
  --report "$OUT/semgrep.json" --db "$OUT/findings.sqlite" --engine semgrep

RUN_ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["run_id"])' "$OUT/ingest.json")
python3 "$REPO/tools/assess_security_findings.py" --repo "$REPO" \
  --db "$OUT/findings.sqlite" --run-id "$RUN_ID" \
  --report "$OUT/semgrep.json" \
  --ground-truth "$REPO/benchmarks/security-validity/ground-truth.json" \
  --excession "$REPO/core/target/debug/excession" --trusted-local true \
  > "$OUT/validity.json"
```

The assessor re-invokes the real `excession security audit` before reading the
candidate table; checks the run ID, commit, engine and original report hash;
requires clean, tracked, Git-blob-exact source and disjoint pinned ground-truth
spans; executes fresh Go tests without network module access; requires all
named positive and negative oracles to pass; and classifies *only* candidate
locations covered by those tests.

The assessor must not run arbitrary/untrusted checkouts on a shared host.
`--trusted-local true` is a deliberate execution authorization, not a sandbox.
For third-party sources, use an isolated no-credentials worker with resource
and egress restrictions.

For a CodeQL report, build the CodeQL database separately in an isolated,
trusted environment, use a pinned query pack and `--format=sarifv2.1.0`, then
run the same Excession ingest/audit/assess flow with `--engine codeql`. An
unlabeled CodeQL rule or alert outside the pinned ground-truth regions remains
unjudged. Do not select query families after inspecting the desired answers.

## Interpretation of an assessment

- `REPRODUCED_LOCAL_BOUNDARY`: a reported source location matches a
  Git-blob-pinned positive case whose independent behavioral test passed.
- `SAFE_CONTROL_HIT`: the scanner reported a bounded negative-control location;
  that is a **demonstrated false positive within this benchmark's stated
  weakness family**, not a generally clean file.
- `UNJUDGED_CANDIDATE`: no local ground truth covers the alert.
- `known_positive_cases_without_hit`: local positive control not observed in
  this run. **Not a recall estimate** unless scanner execution, rule coverage
  and target coverage have separately been established.

No absence of warnings is interpreted as proof of safety. There is no
`CONFIRMED_VULNERABILITY` state here. The assessor's own controlled Go tests
cannot confirm that a real-world alert is reachable, exploitable or material.
A human review and genuine application-level reproduction are further steps.

## Next validity milestone: held-out real cases

Run **both** actual CodeQL and actual Semgrep over a preregistered set of
historical vulnerable/patched revisions plus safe controls, with versioned
rules/query packs and tool binaries. Run each independently of the ground-truth
labels. Freeze report bytes and record scanner versions, duration and scope.
Review each alert with native reproduction and patched-revision regression,
including unsupportable and false-positive findings. Publish TP, FP, unjudged,
and not-observed counts by scanner, weakness family and repository; calculate
precision only over adjudicated hits, and recall only with independently
established scanner coverage. Avoid aggregating repeated warnings about a
single underlying defect into multiple vulnerabilities.

The current Go corpus proves the assessor can distinguish a bounded real
behavioral positive from controls; it does **not** establish CodeQL/Semgrep
precision, recall or superiority on real projects. Those claims require actual
scanner reports and held-out outcomes.

## Developer check

```sh
python3 -m unittest discover -s tests -p test_security_validity.py -v
(cd benchmarks/security-validity && GOPROXY=off GOTOOLCHAIN=local go test -count=1 ./...)
```
