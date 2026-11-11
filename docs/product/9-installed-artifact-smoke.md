# PR 9: Installed release archive end-to-end acceptance

**Status:** plan, test receipt schema and red/pending behavioral test only. No binaries built or published by this scaffold.

Extend the **existing** `.github/workflows/portable-cli.yml` and `tools/test_portable_archive.py` after #79–#84 are implemented. Extract the actual archive into an independent directory with no source checkout or package-side Rust dependencies. Verify checksum, exact version, parser sidecar, Git, and optional runtime capabilities.

Test a pinned fixture through INSTALL → DOCTOR → ANALYZE → IMPACT → EXPLAIN → AUDIT, including honest PARTIAL/UNAVAILABLE cases and source tamper refusal. Support only targets which have actually passed artifact execution. Do not turn normal PR checks into full cross-OS matrices; run full matrix on tags or explicit release validation.

The status field distinguishes planned from run. A valid NOT_RUN receipt does **not** prove that the distributed binary works.

**Auto-ontology:** optional #88–#95 artifacts are not required for CLI smoke. If present, #94 linkage is annotation-only and cannot change core reported edges. No model download or concept compiler is implied by installation.

**TDD:** structural tests now, portable archive smoke pending.

Tracks #83, #84, #101.
