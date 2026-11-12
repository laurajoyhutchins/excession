# PR 9: Installed release archive end-to-end acceptance

**Status:** real installed-archive acceptance belongs to the existing portable CLI workflow and executable smoke harness, not a proposed transcript schema.

Use the **existing** `.github/workflows/portable-cli.yml` and `tools/test_portable_archive.py`. Extract the actual archive into an independent directory with no source checkout or package-side Rust dependencies. Verify checksum, exact version, parser sidecar, Git, and optional runtime capabilities.

Test a pinned fixture through INSTALL → DOCTOR → ANALYZE → IMPACT → EXPLAIN → AUDIT, including honest PARTIAL/UNAVAILABLE cases and source tamper refusal. Support only targets which have actually passed artifact execution. Do not turn normal PR checks into full cross-OS matrices; run full matrix on tags or explicit release validation.

Only actual execution receipts from the installed archive count as smoke-test evidence. A planned or skipped case is not a successful installation.

**Auto-ontology:** optional #88–#95 artifacts are not required for CLI smoke. If present, #94 linkage is annotation-only and cannot change core reported edges. No model download or concept compiler is implied by installation.

**TDD:** run portable archive smoke on the installed artifact; preserve failures and environment details in its test output.

Tracks #83, #84, #101.
