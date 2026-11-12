# PR 6: Evidence-aware impact query

**Status:** contract and tests only. `excession impact --base REV` is not implemented here.

Reuse existing `views::impact` and `views::review` plus neutral `graph deps/path` instead of inventing another graph. A path may be source-observed but still not imply a breakage; SCIP references are not import edges, compiler declarations are not runtime calls.

Impact results refer to exact changed-source receipt, preserve hop-level evidence and distinguish candidate/observed paths. Unknown/unsupported/dynamic inputs stay reported. `safe_to_skip_unlisted_tests` must always be false.

#94 auto-ontology analysis links are **optional annotation-only context**, never a source of new dependency hops. Concept identity and reviewed memberships cannot certify impact or safety.

**Tests:** active structural refusals for proof inflation, test skipping, empty evidence and ontology promotion; pending independently established impact reachability oracle and source replay.

Tracks #80, #81, #84, #94, #101.
