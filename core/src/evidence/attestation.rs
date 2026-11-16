//! Independently verified GitHub-signed artifact claims.
//!
//! The GitHub CLI checks the Sigstore signature, witness timestamp, artifact
//! digest, signer workflow, source commit and source ref. Excession persists
//! that verified *claim* against typed artifact evidence and re-verifies it
//! for audits. A signed claim is not a causal proof that the declared source
//! produced the bytes, nor complete build inputs or an independent rebuild.
use crate::adapter;
use crate::compiler_check::{tool_path, toolchain};
use crate::evidence::build;
use crate::evidence::identity::Subject;
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const SCHEMA: &str = "excession.signed-artifact.v1";
const PRODUCER: &str = "excession:verified-github-artifact-attestation";
const PREDICATE: &str = "https://slsa.dev/provenance/v1";
const POLICY: &str = "GH_CLI_SIGSTORE_SOURCE_SIGNER_PREDICATE_VERIFIED";
const EXECUTION: &str = "SIGNED_WORKFLOW_CLAIM_NOT_REPRODUCED_BUILD";
const MAX_ATTESTATIONS: usize = 30;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct VerifySpec<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub artifact: &'a Path,
    pub signed_repository: &'a str,
    pub signer_workflow: &'a str,
    pub source_ref: &'a str,
    pub trusted_local: bool,
}

fn hex_sha256(value: &str) -> Result<()> {
    Subject::Configuration { sha256: value.to_owned() }.validate()
}

fn exact_fields(value: &Value, allowed: &[&str]) -> Result<()> {
    let fields = value.entries()?;
    if fields.len() != allowed.len()
        || fields.keys().any(|field| !allowed.contains(&field.as_str()))
    {
        return refuse("signed artifact report contains missing or unsupported fields");
    }
    Ok(())
}

fn policy(repo: &str, signer: &str, source_ref: &str) -> Result<()> {
    let segments = repo.split('/').collect::<Vec<_>>();
    if segments.len() != 2
        || segments.iter().any(|part| {
            part.is_empty()
                || part.starts_with('.')
                || !part.bytes().all(|b| b.is_ascii_alphanumeric()
                    || matches!(b, b'_' | b'-' | b'.'))
        })
    {
        return refuse("attestation repository must be an explicit owner/repo");
    }
    let signer = signer.strip_prefix("github.com/").unwrap_or(signer);
    let prefix = format!("{repo}/.github/workflows/");
    let workflow = signer.strip_prefix(&prefix)
        .ok_or("attestation signer must identify a workflow in the selected repository")?;
    if workflow.is_empty()
        || workflow.contains('/')
        || workflow.contains('@')
        || (!workflow.ends_with(".yml") && !workflow.ends_with(".yaml"))
        || workflow.starts_with('.')
        || workflow.bytes().any(|b| !b.is_ascii_alphanumeric()
            && !matches!(b, b'_' | b'-' | b'.'))
    {
        return refuse("invalid explicitly pinned signer workflow path");
    }
    let ref_path = source_ref
        .strip_prefix("refs/heads/")
        .or_else(|| source_ref.strip_prefix("refs/tags/"))
        .ok_or("attestation source ref must be refs/heads/* or refs/tags/*")?;
    git::validate_path(ref_path)?;
    if source_ref.contains(' ') || source_ref.len() > 256 {
        return refuse("invalid attestation source ref");
    }
    Ok(())
}

fn verified_bundle(report: &Value, expected_artifact_sha256: &str) -> Result<Value> {
    let records = report.array_ref()?;
    if records.is_empty() || records.len() > MAX_ATTESTATIONS {
        return refuse("verifier returned no attestations or too many attestations");
    }
    let mut bundles = BTreeMap::new();
    for record in records {
        let verified = record.get("verificationResult")?;
        let signature = verified.get("signature")?;
        if signature.get("certificate")?.entries()?.is_empty()
            || verified.get("verifiedTimestamps")?.array_ref()?.is_empty()
        {
            return refuse("verifier omitted certificate or independent timestamp witness");
        }
        let statement = verified.get("statement")?;
        if statement.get("predicateType")?.str()? != PREDICATE {
            return refuse("verifier returned an unsupported attestation predicate");
        }
        let subject = statement.get("subject")?.array_ref()?;
        if !subject.iter().any(|item| {
            item.get("digest").and_then(|v| v.get("sha256"))
                .and_then(Value::str).ok() == Some(expected_artifact_sha256)
        }) {
            return refuse("verified attestation subject does not match artifact bytes");
        }
        let bundle = record.get("attestation")?.clone();
        if bundle.entries()?.is_empty() {
            return refuse("verified attestation has no original bundle");
        }
        let hash = digest::sha256(bundle.canonical().as_bytes());
        bundles.insert(hash, bundle);
    }
    bundles.into_iter().next().map(|(_, bundle)| bundle)
        .ok_or_else(|| "no verified attestation bundle".to_owned())
}

fn invoke_verifier(
    artifact: &Path,
    signed_repository: &str,
    signer_workflow: &str,
    source_ref: &str,
    source_commit: &str,
) -> Result<(Value, Value)> {
    let tool = tool_path("gh", None)?;
    let toolchain = toolchain(&tool, "gh")?;
    let mut command = Command::new(&tool);
    command
        .arg("attestation")
        .arg("verify")
        .arg(artifact)
        .arg("--repo").arg(signed_repository)
        .arg("--signer-workflow").arg(signer_workflow)
        .arg("--source-ref").arg(source_ref)
        .arg("--source-digest").arg(source_commit)
        .arg("--predicate-type").arg(PREDICATE)
        .arg("--format").arg("json")
        .arg("--limit").arg(MAX_ATTESTATIONS.to_string());
    let output = adapter::run_tool_status(command, "GitHub attestation verification", VERIFY_TIMEOUT)?;
    if output.exit_code != 0 {
        return refuse(format!(
            "GitHub signature, signer, source or artifact verification failed (exit {})",
            output.exit_code
        ));
    }
    let parsed = json::parse(&output.stdout)?;
    let (hash, _) = build::read_artifact(artifact)?;
    let bundle = verified_bundle(&parsed, &hash)?;
    Ok((bundle, toolchain))
}

fn claim(
    commit: &str,
    tree: &str,
    sha256: &str,
    size_bytes: u64,
    signed_repository: &str,
    signer_workflow: &str,
    source_ref: &str,
    verifier: Value,
    bundle: Value,
) -> Result<Value> {
    let subject_id = Subject::Artifact {
        sha256: sha256.to_owned(), size_bytes,
    }.id()?;
    let bundle_sha = digest::sha256(bundle.canonical().as_bytes());
    obj(&[
        ("schema", Value::string(SCHEMA)),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("source_ref", Value::string(source_ref)),
        ("signed_repository", Value::string(signed_repository)),
        ("signer_workflow", Value::string(signer_workflow)),
        ("artifact", obj(&[
            ("sha256", Value::string(sha256)),
            ("size_bytes", Value::number(size_bytes)),
            ("subject_id", Value::string(&subject_id)),
        ])),
        ("attestation_bundle_sha256", Value::string(bundle_sha)),
        ("attestation_bundle", bundle),
        ("verifier", verifier),
        ("verification_policy", Value::string(POLICY)),
        ("workflow_execution", Value::string(EXECUTION)),
        ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
        ("dependency_closure", Value::string("NOT_ESTABLISHED")),
        ("rea", Value::string("NOT_ESTABLISHED")),
    ]).stamp_id("report_id")
}

fn validate_claim(report: &Value) -> Result<()> {
    exact_fields(report, &[
        "schema", "source_commit", "source_tree", "source_ref",
        "signed_repository", "signer_workflow", "artifact",
        "attestation_bundle_sha256", "attestation_bundle", "verifier",
        "verification_policy", "workflow_execution",
        "source_artifact_relation", "dependency_closure", "rea", "report_id",
    ])?;
    if report.get("schema")?.str()? != SCHEMA
        || !report.content_id_matches("report_id")?
        || report.get("verification_policy")?.str()? != POLICY
        || report.get("workflow_execution")?.str()? != EXECUTION
        || report.get("source_artifact_relation")?.str()? != "NOT_ESTABLISHED"
        || report.get("dependency_closure")?.str()? != "NOT_ESTABLISHED"
        || report.get("rea")?.str()? != "NOT_ESTABLISHED"
    {
        return refuse("signed artifact report makes unsupported claims or has invalid digest");
    }
    policy(
        report.get("signed_repository")?.str()?,
        report.get("signer_workflow")?.str()?,
        report.get("source_ref")?.str()?,
    )?;
    git::oid(report.get("source_commit")?.str()?)?;
    git::oid(report.get("source_tree")?.str()?)?;
    let artifact = report.get("artifact")?;
    exact_fields(artifact, &["sha256", "size_bytes", "subject_id"])?;
    let sha = artifact.get("sha256")?.str()?;
    hex_sha256(sha)?;
    let bytes = artifact.get("size_bytes")?.integer()?;
    if !(1..=16 * 1024 * 1024).contains(&bytes) {
        return refuse("signed artifact size exceeds declared byte limits");
    }
    let subject = Subject::Artifact { sha256: sha.into(), size_bytes: bytes as u64 }.id()?;
    if artifact.get("subject_id")?.str()? != subject {
        return refuse("signed artifact subject identity mismatch");
    }
    let bundle_sha = report.get("attestation_bundle_sha256")?.str()?;
    hex_sha256(bundle_sha)?;
    if digest::sha256(report.get("attestation_bundle")?.canonical().as_bytes()) != bundle_sha {
        return refuse("signed artifact original bundle digest mismatch");
    }
    report.get("attestation_bundle")?.entries()?;
    let verifier = report.get("verifier")?;
    exact_fields(verifier, &["name", "executable", "launcher_sha256", "version", "identity_scope"])?;
    if verifier.get("name")?.str()? != "gh"
        || verifier.get("identity_scope")?.str()? != "LAUNCHER_AND_VERSION_NOT_TRANSITIVE_TOOLCHAIN"
        || !Path::new(verifier.get("executable")?.str()?).is_absolute()
        || verifier.get("version")?.str()?.is_empty()
    {
        return refuse("signed artifact verifier metadata invalid");
    }
    hex_sha256(verifier.get("launcher_sha256")?.str()?)?;
    Ok(())
}

fn batch(report: &Value) -> Result<Batch> {
    validate_claim(report)?;
    let raw_json = report.canonical();
    let receipt = Receipt {
        producer: PRODUCER.to_owned(),
        version: "1".to_owned(),
        native_id: report.get("report_id")?.str()?.to_owned(),
        raw_json,
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    let artifact = report.get("artifact")?;
    Ok(Batch {
        source_commit: report.get("source_commit")?.str()?.to_owned(),
        receipts: vec![receipt],
        observations: vec![Observation {
            receipt_id,
            subject_id: artifact.get("subject_id")?.str()?.to_owned(),
            native_kind: "GITHUB_SIGNED_ARTIFACT_CLAIM".to_owned(),
            payload: obj(&[
                ("artifact", artifact.clone()),
                ("source_commit", report.get("source_commit")?.clone()),
                ("signed_repository", report.get("signed_repository")?.clone()),
                ("signer_workflow", report.get("signer_workflow")?.clone()),
                ("attestation_bundle_sha256", report.get("attestation_bundle_sha256")?.clone()),
                ("verification_policy", Value::string(POLICY)),
                ("workflow_execution", Value::string(EXECUTION)),
                ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
                ("dependency_closure", Value::string("NOT_ESTABLISHED")),
                ("rea", Value::string("NOT_ESTABLISHED")),
            ]),
        }],
    })
}

fn with_snapshot(mut report: Value, snapshot: &str) -> Result<Value> {
    report.entries_mut()?.insert("snapshot_id".into(), Value::string(snapshot));
    Ok(report)
}

/// Validate a signed claim with GitHub's Sigstore verifier; persist the exact
/// verified bundle in the existing evidence store. No artifact is executed.
pub fn verify(db: &Db, spec: &VerifySpec<'_>) -> Result<Value> {
    if !spec.trusted_local {
        return refuse("GitHub verification requires --trusted-local true for the local verifier");
    }
    policy(spec.signed_repository, spec.signer_workflow, spec.source_ref)?;
    let repo = spec.repo.canonicalize().map_err(|e| format!("resolve source repository: {e}"))?;
    git::verify_repo(&repo)?;
    let commit = git::rev(&repo, &format!("{}^{{commit}}", spec.revision))?;
    let tree = git::rev(&repo, &format!("{commit}^{{tree}}"))?;
    let artifact = spec.artifact.canonicalize().map_err(|e| format!("resolve artifact: {e}"))?;
    let (sha, size) = build::read_artifact(&artifact)?;
    let (bundle, verifier) = invoke_verifier(
        &artifact, spec.signed_repository, spec.signer_workflow, spec.source_ref, &commit
    )?;
    if build::read_artifact(&artifact)? != (sha.clone(), size) {
        return refuse("artifact bytes changed during GitHub verification");
    }
    let report = claim(
        &commit, &tree, &sha, size, spec.signed_repository,
        spec.signer_workflow, spec.source_ref, verifier, bundle
    )?;
    let snapshot = evidence_store::persist(db, &batch(&report)?)?;
    with_snapshot(report, &snapshot)
}

/// Independently re-run cryptographic signature verification against the live
/// GitHub attestation API and check the exact original stored bundle. This
/// requires a valid GitHub CLI and network access at audit time.
pub fn audit(
    db: &Db, repo: &Path, revision: &str, artifact: &Path, snapshot: &str
) -> Result<Value> {
    let stored = evidence_store::audit(db, snapshot)?;
    let rows = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot)],
    )?;
    if rows.len() != 1 {
        return refuse("signed artifact snapshot must contain exactly one receipt");
    }
    let receipt = json::parse(rows[0][0].as_deref()
        .ok_or("missing original signed artifact receipt")?.as_bytes())?;
    if receipt.get("producer")?.str()? != PRODUCER
        || receipt.get("version")?.str()? != "1"
    {
        return refuse("not a GitHub verified artifact claim receipt");
    }
    let raw = receipt.get("raw_json")?.str()?;
    let claim = json::parse(raw.as_bytes())?;
    if raw != claim.canonical()
        || receipt.get("native_id")?.str()? != claim.get("report_id")?.str()?
        || evidence_store::manifest(&batch(&claim)?)? != stored
    {
        return refuse("signed artifact original evidence differs from canonical receipt");
    }
    let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    if claim.get("source_commit")?.str()? != commit
        || claim.get("source_tree")?.str()? != tree
    {
        return refuse("signed artifact source is not the selected Git revision");
    }
    let (sha, size) = build::read_artifact(artifact)?;
    if claim.get("artifact")?.get("sha256")?.str()? != sha
        || claim.get("artifact")?.get("size_bytes")?.integer()? != size as i64
    {
        return refuse("signed artifact no longer matches its original bytes");
    }
    let (new_bundle, _) = invoke_verifier(
        artifact,
        claim.get("signed_repository")?.str()?,
        claim.get("signer_workflow")?.str()?,
        claim.get("source_ref")?.str()?,
        &commit,
    )?;
    if digest::sha256(new_bundle.canonical().as_bytes())
        != claim.get("attestation_bundle_sha256")?.str()?
    {
        return refuse("fresh GitHub verification did not recover the original signed bundle");
    }
    with_snapshot(claim, snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_result(artifact_sha: &str) -> Value {
        Value::array(vec![obj(&[
            ("attestation", obj(&[("signer", Value::string("a real bundle is checked by gh"))])),
            ("verificationResult", obj(&[
                ("signature", obj(&[("certificate", obj(&[
                    ("subject", Value::string("parsed X.509 identity")),
                ]))])),
                ("verifiedTimestamps", Value::array(vec![obj(&[
                    ("time", Value::string("from a signature witness")),
                ])])),
                ("statement", obj(&[
                    ("predicateType", Value::string(PREDICATE)),
                    ("subject", Value::array(vec![obj(&[
                        ("digest", obj(&[("sha256", Value::string(artifact_sha))])),
                    ])])),
                ])),
            ])),
        ])])
    }

    #[test]
    fn verifier_output_requires_subject_predicate_timestamp_and_certificate() {
        let sha = "a".repeat(64);
        let value = fixture_result(&sha);
        assert!(verified_bundle(&value, &sha).is_ok());
        assert!(verified_bundle(&value, &"b".repeat(64)).is_err());
        let mut other = value.clone();
        other.array_ref().unwrap(); // verify shape before mutating the specimen
        let mut first = other.array_ref().unwrap()[0].clone();
        first.entries_mut().unwrap().get_mut("verificationResult").unwrap()
            .entries_mut().unwrap().get_mut("verifiedTimestamps")
            .map(|field| *field = Value::array(Vec::new()));
        other = Value::array(vec![first]);
        assert!(verified_bundle(&other, &sha).is_err());
    }

    #[test]
    fn signer_and_git_source_ref_are_explicit_and_bounded() {
        assert!(policy("laurajoyhutchins/excession",
            "github.com/laurajoyhutchins/excession/.github/workflows/smoke.yml",
            "refs/heads/main").is_ok());
        assert!(policy("owner/repo",
            "unrelated/repo/.github/workflows/smoke.yml", "refs/heads/main").is_err());
        assert!(policy("owner/repo",
            "owner/repo/.github/workflows/smoke.yml", "main").is_err());
        assert!(policy("owner/repo",
            "owner/repo/.github/workflows/../smoke.yml", "refs/heads/main").is_err());
    }

    #[test]
    fn self_hash_does_not_turn_claim_into_proven_build() {
        let v = claim(
            &"a".repeat(40), &"b".repeat(40), &"c".repeat(64), 16,
            "owner/repo", "owner/repo/.github/workflows/build.yml", "refs/heads/main",
            obj(&[
                ("name", Value::string("gh")),
                ("executable", Value::string("/usr/bin/gh")),
                ("launcher_sha256", Value::string("d".repeat(64))),
                ("version", Value::string("gh 2.x")),
                ("identity_scope", Value::string("LAUNCHER_AND_VERSION_NOT_TRANSITIVE_TOOLCHAIN")),
            ]),
            obj(&[("bundle", Value::string("test-only"))]),
        ).unwrap();
        assert!(validate_claim(&v).is_ok());
        assert_eq!(v.get("source_artifact_relation").unwrap().str().unwrap(), "NOT_ESTABLISHED");
        assert_eq!(v.get("dependency_closure").unwrap().str().unwrap(), "NOT_ESTABLISHED");
        let mut raised = v.clone();
        raised.entries_mut().unwrap().remove("report_id");
        raised.entries_mut().unwrap().insert(
            "source_artifact_relation".into(), Value::string("PROVEN")
        );
        let raised = raised.stamp_id("report_id").unwrap();
        assert!(validate_claim(&raised).is_err());
    }
}
