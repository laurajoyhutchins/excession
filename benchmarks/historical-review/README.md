# Historical coding-agent PR review experiment

This is a deterministic, Rust-native **experiment harness**, not an agent service.
It prepares a pre-fix workspace, projects the same source analysis into four
information conditions, records externally executed agent reviews, and counts
paired merge-gate decisions against separately held oracle receipts.

No model API key or agent tokens are needed to prepare or test the infrastructure.

## Conditions

| Arm | Reviewer sees |
| --- | --- |
| \`baseline\` | Git candidate and its ancestors, task and ordinary developer tooling |
| \`legacy_paths\` | The same candidate plus Excession's bounded simple dependency paths |
| \`graph\` | The same candidate plus a flat distinct-source set/ranks, without witness edges |
| \`excession\` | The same candidate plus the compact provenance-bearing witness forest |

The three analysis arms derive from **one invocation** of the current
\`agent::verify\` implementation, independently for each assignment. Source
coverage remains partial. They do not claim safe-to-skip tests.

The agent-visible assignment directory contains \`workspace/\`, \`prompt.txt\`,
\`assignment.json\` and, for non-baseline arms, \`evidence.json\`. Only the
candidate and its **ancestors** are fetched into a fresh Git object database.
There is no configured upstream remote. The repaired commit, independent
oracle and private study manifest must never enter the agent's runtime.

**Important:** Git object isolation does not prevent an agent with unrestricted
filesystem/network access from opening the original source checkout or
searching for public fixes. Put the agent inside a filesystem and network sandbox
that exposes only its assignment directory, prohibit remote access, and audit
tool usage. Do not put private study paths in the task prompt.

## Private study manifest

A study JSON contains \`schema="excession.historical-review-study.v1"\`, a safe
\`id\`, \`repetitions\` (1–10), and \`cases\`. Each case must include:

- \`id\`, absolute local \`repo_path\`, pinned lowercase SHA-1 \`base\` and
  \`candidate\`, bounded \`task\` and source \`roots\` CSV;
- \`truth\` = \`REGRESSION\` or \`CLEAN\`; only regression cases have a
  \`repair\` commit descending from the candidate;
- \`oracles.base\` and \`oracles.candidate\`; regression cases also require
  \`oracles.repair\`. Each entry has \`path\` (absolute receipt JSON filename) and
  \`sha256\` (content digest).

Each **private oracle receipt** has the following fields, captured independently
before an agent review. It is **not** copied into the agent assignment.

\`\`\`json
{
  "schema": "excession.historical-review-oracle.v1",
  "commit": "<full lowercase commit SHA-1>",
  "tree": "<full lowercase tree SHA-1>",
  "command": ["cargo", "test", "--locked", "regression_name"],
  "exit_code": 0,
  "stdout_sha256": "<SHA-256 of raw stdout>",
  "stderr_sha256": "<SHA-256 of raw stderr>",
  "runner": "independent-runner-and-environment-identity"
}
\`\`\`

All three receipts must refer to the same command, bind the exact Git commit
and tree, and report A=pass, B=fail, C=pass. Clean controls require A=pass and
B=pass, without C. The sealed study SHA-256 is embedded in every prepared assignment and each
attempt receipt, preventing silent replacement of hidden labels after preparation.
Receipt hashes reject mutation, but **do not prove that a
claimed command executed**. Preserve original logs, pinned toolchains and an
independent execution attestation; independently audit a sample of cases. A
developer-authored assertion alone is not accepted as a real-world regression.

## Commands

Build the Rust CLI once with \`cargo build --locked --manifest-path core/Cargo.toml\`.
Then, from a **trusted preparation host**:

\`\`\`sh
BIN=core/target/debug/excession

$BIN benchmark review-prepare \
  --study /private/pilot.json --case case-01 \
  --condition baseline --repetition 1 --out /tmp/pilot/case-01-baseline

$BIN benchmark review-prepare \
  --study /private/pilot.json --case case-01 \
  --condition excession --repetition 1 --out /tmp/pilot/case-01-excession

# Run agents separately, one clean isolated runtime per case/arm/repetition.
# Preserve the transcript and usage receipt. No model is invoked above.

$BIN benchmark review-record \
  --assignment /tmp/pilot/case-01-excession/assignment.json \
  --transcript /private/transcripts/case-01-excession.txt \
  --decision BLOCK --input-tokens 23000 --output-tokens 6100 \
  --out /private/attempts/case-01-excession.json

$BIN benchmark review-evaluate \
  --study /private/pilot.json --attempts /private/attempts \
  --out /private/pilot-results.json
\`\`\`

An agent's decision is \`APPROVE\`, \`BLOCK\`, or \`ABSTAIN\`. Omit **both**
token flags if usage is not measured. \`review-record\` computes receipt,
assignment and transcript hashes; it refuses overwriting an existing attempt.
\`review-evaluate\` refuses duplicates and changed artifacts. Missing attempts
stay in the denominator. Per-arm caller-reported token totals and unmeasured
attempt counts are separate, not zero-filled. Its \`CORRECT_GATE\` measure is **only a merge-decision
proxy**: blocking a bad commit is not independently adjudicated as a valid
explanation, and producing a repaired implementation is out of scope.

For the first pilot, compare baseline and Excession on **paired** regressions
and clean controls. Report escapes, false blocks, abstentions, missing cells,
cost and oracle validity separately. Do not compute a misleading speedup from
different operations or a victory rate from only completed runs. Run the same
agent snapshot, prompts, permissions, budgets and runtime environment for each
arm; randomize arm order and prevent carryover between cases.

## CI and status

The \`historical_review\` Rust tests construct a three-commit Git regression,
exercise all four evidence arms, verify that the repaired commit is not
fetchable from the blinded checkout, round-trip four attempt receipts, and
refuse a tampered transcript. Existing \`Verify\` CI runs these tests alongside
the rest of the Rust package. No remote agent calls are made in CI.

**Not yet supplied:** an independently adjudicated real-regression cohort, an
isolated model runner, independently verified model usage receipts, or agent
effectiveness measurements. The infrastructure intentionally does not invent
those observations.
