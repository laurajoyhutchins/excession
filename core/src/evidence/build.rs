//! Replayable build inputs and artifact bytes, not self-attested build execution.
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INPUTS: usize = 128;
const PRODUCER: &str = "excession:build-provenance";
const FORMAT: &str = "excession.build-provenance.v1";

fn sha256(value: &str) -> Result<()> {
    Subject::Configuration {
        sha256: value.to_owned(),
    }
    .validate()
}
fn representation(value: &str) -> Result<()> {
    if matches!(
        value,
        "EXECUTABLE" | "OBJECT" | "ARCHIVE" | "WASM" | "OTHER"
    ) {
        Ok(())
    } else {
        refuse("unsupported build artifact representation")
    }
}
fn read_artifact(path: &Path) -> Result<(String, u64)> {
    let file = std::fs::File::open(path).map_err(|e| format!("artifact open: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_ARTIFACT_BYTES {
        return refuse("artifact must be a regular file within the 16 MiB budget");
    }
    let mut bytes = Vec::new();
    file.take(MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("artifact read: {e}"))?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return refuse("artifact changed or exceeded byte limit during read");
    }
    Ok((digest::sha256(&bytes), bytes.len() as u64))
}
fn source_doc(source: &SourceRef) -> Value {
    obj(&[
        ("commit", Value::string(&source.commit)),
        ("tree", Value::string(&source.tree)),
        ("path", Value::string(&source.path)),
        ("blob", Value::string(&source.blob)),
        ("mode", Value::string(&source.mode)),
    ])
}
struct Prepared {
    batch: Batch,
    claim: Value,
    receipt_id: String,
    artifact_subject_id: String,
}
fn prepare(
    repo: &Path,
    revision: &str,
    artifact: &Path,
    inputs: &[&str],
    toolchain: &str,
    configuration: &str,
    kind: &str,
) -> Result<Prepared> {
    git::verify_repo(repo)?;
    let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    sha256(toolchain)?;
    sha256(configuration)?;
    representation(kind)?;
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return refuse("build evidence requires 1..128 declared Git inputs");
    }
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for path in inputs {
        if !seen.insert(*path) {
            return refuse("duplicate declared build input");
        }
        sources.push(SourceRef::from_git(repo, &commit, path)?);
    }
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    let (sha, size) = read_artifact(artifact)?;
    let artifact_subject_id = Subject::Artifact {
        sha256: sha.clone(),
        size_bytes: size,
    }
    .id()?;
    let claim = obj(&[
        ("schema", Value::string(FORMAT)),
        ("source_commit", Value::string(&commit)),
        ("source_tree", Value::string(&tree)),
        (
            "artifact",
            obj(&[
                ("sha256", Value::string(&sha)),
                ("size_bytes", Value::number(size)),
                ("representation", Value::string(kind)),
            ]),
        ),
        (
            "build",
            obj(&[
                ("toolchain_digest", Value::string(toolchain)),
                ("configuration_digest", Value::string(configuration)),
                (
                    "source_inputs",
                    Value::array(sources.iter().map(source_doc)),
                ),
                (
                    "execution_authority",
                    Value::string("DECLARED_RECIPE_UNATTESTED"),
                ),
            ]),
        ),
        ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
        (
            "execution_security",
            Value::string("UNTRUSTED_BUILDS_REQUIRE_ISOLATION"),
        ),
    ]);
    let receipt = Receipt {
        producer: PRODUCER.into(),
        version: "1".into(),
        native_id: claim.sha256(),
        raw_json: claim.canonical(),
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    let mut observations = vec![Observation {
        receipt_id: receipt_id.clone(),
        subject_id: artifact_subject_id.clone(),
        native_kind: "BUILD_ARTIFACT_BYTES".into(),
        payload: obj(&[
            ("artifact", claim.get("artifact")?.clone()),
            ("relation", Value::string("ARTIFACT_BYTES_OBSERVED")),
        ]),
    }];
    for source in sources {
        observations.push(Observation {
            receipt_id: receipt_id.clone(),
            subject_id: Subject::Source(source.clone()).id()?,
            native_kind: "BUILD_INPUT_DECLARATION".into(),
            payload: obj(&[
                ("source", source_doc(&source)),
                ("artifact_subject_id", Value::string(&artifact_subject_id)),
                ("relation", Value::string("DECLARED_BUILD_INPUT_ONLY")),
            ]),
        });
    }
    Ok(Prepared {
        batch: Batch {
            source_commit: commit,
            receipts: vec![receipt],
            observations,
        },
        claim,
        receipt_id,
        artifact_subject_id,
    })
}
fn report(prepared: &Prepared, snapshot: &str) -> Result<Value> {
    let mut result = prepared.claim.clone();
    let body = result.entries_mut()?;
    body.insert("snapshot_id".into(), Value::string(snapshot));
    body.insert("receipt_id".into(), Value::string(&prepared.receipt_id));
    body.insert(
        "artifact_subject_id".into(),
        Value::string(&prepared.artifact_subject_id),
    );
    body.insert("artifact_bytes_verified".into(), Value::Bool(true));
    body.insert("git_inputs_verified".into(), Value::Bool(true));
    body.insert("build_execution_verified".into(), Value::Bool(false));
    body.insert(
        "status".into(),
        Value::string("GIT_INPUTS_AND_ARTIFACT_BYTES_REPLAYED"),
    );
    let id = result.sha256();
    result
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(result)
}
/// Caller-owned description; keep the established Git/SQLite store authoritative.
pub struct BuildSpec<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub artifact: &'a Path,
    pub inputs: &'a [&'a str],
    pub toolchain: &'a str,
    pub configuration: &'a str,
    pub kind: &'a str,
}
/// Persist immutable source and artifact facts in the shared evidence store.
/// Never execute untrusted build commands on the host.
pub fn record(db: &Db, spec: &BuildSpec<'_>) -> Result<Value> {
    let prepared = prepare(
        spec.repo,
        spec.revision,
        spec.artifact,
        spec.inputs,
        spec.toolchain,
        spec.configuration,
        spec.kind,
    )?;
    let snapshot = evidence_store::persist(db, &prepared.batch)?;
    report(&prepared, &snapshot)
}
/// Rebuild the original snapshot from the original receipt, pinned Git objects,
/// and freshly read artifact bytes. Refuse any disagreement.
pub fn audit(
    db: &Db,
    repo: &Path,
    revision: &str,
    artifact: &Path,
    snapshot: &str,
) -> Result<Value> {
    db.read_transaction(|db| {
        let existing = evidence_store::audit(db, snapshot)?;
        let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
        if existing.get("source_commit")?.str()? != commit {
            return refuse("build provenance snapshot source revision mismatch");
        }
        let rows = db.query(
            "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
            &[Param::Text(snapshot)],
        )?;
        if rows.len() != 1 {
            return refuse("build snapshot requires exactly one original producer receipt");
        }
        let receipt = json::parse(
            rows[0][0]
                .as_deref()
                .ok_or("missing original receipt document")?
                .as_bytes(),
        )?;
        if receipt.get("producer")?.str()? != PRODUCER || receipt.get("version")?.str()? != "1" {
            return refuse("not an Excession build provenance receipt");
        }
        let raw = receipt.get("raw_json")?.str()?;
        let claim = json::parse(raw.as_bytes())?;
        if raw != claim.canonical()
            || claim.get("schema")?.str()? != FORMAT
            || claim.entries()?.len() != 7
        {
            return refuse("build receipt is not canonical or contains unsupported fields");
        }
        let build = claim.get("build")?;
        if build.entries()?.len() != 4 {
            return refuse("unsupported build claim extensions");
        }
        let inputs = build.get("source_inputs")?.array_ref()?;
        if inputs.is_empty() || inputs.len() > MAX_INPUTS {
            return refuse("bad number of recorded source inputs");
        }
        let paths = inputs
            .iter()
            .map(|item| item.get("path").and_then(Value::str))
            .collect::<Result<Vec<_>>>()?;
        let prepared = prepare(
            repo,
            &commit,
            artifact,
            &paths,
            build.get("toolchain_digest")?.str()?,
            build.get("configuration_digest")?.str()?,
            claim.get("artifact")?.get("representation")?.str()?,
        )?;
        if prepared.claim != claim
            || prepared.receipt_id != receipt.get("receipt_id")?.str()?
            || evidence_store::manifest(&prepared.batch)? != existing
        {
            return refuse("artifact bytes, Git inputs, or build observations changed");
        }
        report(&prepared, snapshot)
    })
}
