# Signed artifact claims

Excession can verify a GitHub artifact attestation through the installed
GitHub CLI and record the verified claim alongside the existing Git and
artifact evidence. A signature is evidence of the **workflow identity and
artifact digest**, not proof that every dependency was known or that the
declared source actually caused the output bytes.

This is distinct from `evidence build-record`, which records exact Git
source identities and artifact bytes without a signed executor.

## Record and reverify

The artifact must already exist, and the source Git commit must exist in the
local checkout. Use the precise repository, signer workflow and expected
source ref, chosen independently of the artifact.

```sh
excession evidence signed-artifact \
  --repo . --rev COMMIT --artifact /path/to/application \
  --signed-repo owner/repo \
  --signer-workflow github.com/owner/repo/.github/workflows/build.yml \
  --source-ref refs/heads/main \
  --db /tmp/excession-evidence.sqlite --trusted-local true

excession inspect signed-artifact \
  --repo . --rev COMMIT --artifact /path/to/application \
  --db /tmp/excession-evidence.sqlite --snapshot SNAPSHOT_ID
```

The first command invokes the locally installed `gh attestation verify` with
`--repo`, `--signer-workflow`, `--source-digest`, `--source-ref`,
`--predicate-type https://slsa.dev/provenance/v1`, and `--format json`.
The GitHub CLI independently checks Sigstore signature and provenance,
while Excession checks the source Git commit/tree and the actual artifact
SHA-256. Verification refuses if there is no matching original signed
bundle, certificate, or independently witnessed timestamp.

The original signed bundle and verifier metadata are persisted as an
immutable `excession.signed-artifact.v1` receipt with a typed Artifact
subject and `GITHUB_SIGNED_ARTIFACT_CLAIM` native observation in the
existing SQLite evidence store. The original bundle SHA-256 is retained.

The second command audits the original evidence against the same Git
revision and unchanged artifact, then **reruns** signature verification
through the live GitHub API. It must recover the same signed bundle. It
does not trust a self-hashed JSON receipt as cryptographic proof.

Requirements and constraints:

- Install and authenticate **GitHub CLI 2.102.0 or newer** with access to
  the repository's artifact attestations. Earlier versions have known
  signer-workflow prefix and source-ref matching defects, and Excession
  refuses them. See [GitHub's signer advisory](https://github.com/cli/cli/security/advisories/GHSA-wjmr-j3rp-mh2g).
  The artifact is limited to 16 MiB by the existing build artifact evidence
  contract. Store the SQLite database outside the checkout.
- `--trusted-local true` authorizes running the installed local GitHub CLI.
  This is not independent verification of the local verifier binary or a
  sandbox. The receipt records the verifier launcher hash/version.
- Auditing needs network access to the GitHub API and a current trusted
  GitHub CLI. A removed, inaccessible, or invalid attestation will refuse
  even if an old receipt exists.
- Only a workflow in the explicitly selected repository can be accepted
  by this first policy. The commit and source ref are both pinned.
- A valid signing identity is still a workflow **claim**. A compromised
  workflow can emit a false predicate. The signer workflow and its allowed
  dependencies must be trusted independently. For stronger isolation,
  use a separately controlled, pinned reusable builder.

The receipt always leaves `source_artifact_relation`,
`dependency_closure`, and `rea` as `NOT_ESTABLISHED`. Complete
transitive inputs, reproducible isolated builds, and an independent
source-to-artifact causality demonstration remain open in
[issue #181](https://github.com/laurajoyhutchins/excession/issues/181).

## Real fixture

The
[GitHub Actions acceptance workflow](../../.github/workflows/artifact-attestation-smoke.yml)
compiles a committed dependency-free Rust fixture, signs it with a GitHub
OIDC-backed Sigstore attestation, verifies it through Excession using the
expected source SHA and signer workflow, reaudits the original signed
bundle, and refuses a modified binary. It runs on trusted `main` pushes
to the relevant files and avoids broad execution privileges or a new
attestation service.
