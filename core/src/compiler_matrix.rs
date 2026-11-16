//! A Git-declared collection of native compiler checks with independently
//! replayable child receipts. The matrix is explicit, not a claim that every
//! possible build feature, target, dependency, or configuration was inspected.
use crate::compiler_check::{self, Request, exact_fields, source_doc, validate_configuration_name};
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MANIFEST_SCHEMA: &str = "excession.compiler-matrix-config.v1";
const REPORT_SCHEMA: &str = "excession.compiler-matrix.v1";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CHECKS: usize = 16;
const MAX_INPUTS_PER_CHECK: usize = 32;

pub struct MatrixRequest<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub manifest: &'a str,
    pub rust_tool: Option<&'a str>,
    pub typescript_tool: Option<&'a str>,
    pub python_tool: Option<&'a str>,
    pub trusted_local: bool,
}

#[derive(Clone)]
struct Check {
    language: String,
    configuration: String,
    configuration_source: SourceRef,
    source_prefix: Option<String>,
    declared_inputs: Vec<SourceRef>,
}

/// Cache exact Git lookups within one matrix parse. Every unique source is
/// still authenticated through SourceRef::from_git at the pinned revision.
fn source_cached(
    root: &Path,
    commit: &str,
    path: &str,
    cache: &mut BTreeMap<String, SourceRef>,
) -> Result<SourceRef> {
    if let Some(source) = cache.get(path) {
        return Ok(source.clone());
    }
    let source = SourceRef::from_git(root, commit, path)?;
    cache.insert(path.to_owned(), source.clone());
    Ok(source)
}

fn git_file(root: &Path, commit: &str, path: &str) -> Result<(SourceRef, Vec<u8>)> {
    let source = SourceRef::from_git(root, commit, path)?;
    let mut reader = git::BlobReader::open(root)?;
    let contents = reader.read_capped(&source.blob, MAX_MANIFEST_BYTES)?;
    reader.finish()?;
    Ok((source, contents))
}

fn live_file_matches(root: &Path, source: &SourceRef) -> Result<()> {
    let path = root
        .join(&source.path)
        .canonicalize()
        .map_err(|e| format!("declared compiler input missing from checkout: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return refuse("declared compiler input resolves outside the Git checkout");
    }
    let metadata = fs::metadata(&path).map_err(|e| format!("stat declared compiler input: {e}"))?;
    if metadata.len() > MAX_INPUT_BYTES {
        return refuse("declared compiler input exceeds 16 MiB");
    }
    let bytes = fs::read(&path).map_err(|e| format!("read declared compiler input: {e}"))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES || digest::git_blob(&bytes) != source.blob {
        return refuse("declared compiler input disagrees with Git, or exceeds 16 MiB");
    }
    Ok(())
}

fn parse(root: &Path, commit: &str, source: &SourceRef, bytes: &[u8]) -> Result<Vec<Check>> {
    let doc = json::parse(bytes)?;
    exact_fields(&doc, &["schema", "checks"])?;
    if doc.get("schema")?.str()? != MANIFEST_SCHEMA {
        return refuse("unsupported compiler matrix manifest version");
    }
    let inputs = doc.get("checks")?.array_ref()?;
    if inputs.is_empty() || inputs.len() > MAX_CHECKS {
        return refuse("compiler matrix must declare 1..16 checks");
    }
    let mut seen = BTreeSet::new();
    let mut source_cache = BTreeMap::new();
    let mut prepared = Vec::new();
    for item in inputs {
        exact_fields(
            item,
            &[
                "language",
                "configuration",
                "source_prefix",
                "declared_inputs",
            ],
        )?;
        let language = item.get("language")?.str()?;
        let config = item.get("configuration")?.str()?;
        validate_configuration_name(language, config)?;
        let prefix = match item.get("source_prefix")? {
            Value::Null => None,
            v if v.str()? == "." => None,
            v => Some(v.str()?.to_owned()),
        };
        if language != "python" && prefix.is_some() {
            return refuse("compiler matrix source_prefix is only supported for Python");
        }
        if let Some(ref prefix) = prefix
            && prefix != "."
        {
            git::validate_path(prefix)?;
        }
        if !seen.insert((language.to_owned(), config.to_owned(), prefix.clone())) {
            return refuse("duplicate compiler matrix configuration and scope");
        }
        // A tracked source configuration and every declared input is replayed
        // independently from Git, not trusted from the manifest's assertions.
        let cfg = source_cached(root, commit, config, &mut source_cache)?;
        if cfg.tree != source.tree {
            return refuse("compiler matrix contains a configuration from another tree");
        }
        let paths = item.get("declared_inputs")?.array_ref()?;
        if paths.len() > MAX_INPUTS_PER_CHECK {
            return refuse("compiler matrix has more than 32 declared inputs per check");
        }
        let mut unique = BTreeSet::new();
        let mut refs = Vec::new();
        for path in paths {
            let path = path.str()?;
            if !unique.insert(path.to_owned()) {
                return refuse("duplicate declared input in compiler matrix");
            }
            if path == config || path == source.path {
                return refuse("compiler matrix declared_inputs must not repeat config or matrix");
            }
            refs.push(source_cached(root, commit, path, &mut source_cache)?);
        }
        refs.sort_by(|a, b| a.path.cmp(&b.path));
        prepared.push(Check {
            language: language.to_owned(),
            configuration: config.to_owned(),
            configuration_source: cfg,
            source_prefix: prefix,
            declared_inputs: refs,
        });
    }
    Ok(prepared)
}

fn read_manifest(root: &Path, commit: &str, path: &str) -> Result<(SourceRef, Vec<Check>)> {
    let (source, bytes) = git_file(root, commit, path)?;
    let checks = parse(root, commit, &source, &bytes)?;
    Ok((source, checks))
}

fn entry(check: &Check, receipt: &Value) -> Result<Value> {
    if receipt.get("schema")?.str()? != "excession.compiler-check.v1"
        || receipt.get("language")?.str()? != check.language
        || receipt.get("configuration")? != &source_doc(&check.configuration_source)
        || receipt.get("source_scope")?.str()? != check.source_prefix.as_deref().unwrap_or(".")
    {
        return refuse("compiler matrix child receipt does not match declared configuration");
    }
    Ok(obj(&[
        ("language", Value::string(&check.language)),
        ("configuration", receipt.get("configuration")?.clone()),
        ("source_scope", receipt.get("source_scope")?.clone()),
        (
            "declared_inputs",
            Value::array(check.declared_inputs.iter().map(source_doc)),
        ),
        ("status", receipt.get("status")?.clone()),
        ("check_report_id", receipt.get("report_id")?.clone()),
        ("check_snapshot_id", receipt.get("snapshot_id")?.clone()),
    ]))
}

fn build_report(source: &SourceRef, checks: Vec<Value>) -> Result<Value> {
    let passed = checks
        .iter()
        .all(|v| v.get("status").ok().and_then(|v| v.str().ok()) == Some("PASS"));
    obj(&[
        ("schema", Value::string(REPORT_SCHEMA)),
        ("source_commit", Value::string(&source.commit)),
        ("source_tree", Value::string(&source.tree)),
        ("manifest", source_doc(source)),
        ("checks", Value::array(checks)),
        (
            "status",
            Value::string(if passed { "PASS" } else { "FAIL" }),
        ),
        (
            "coverage",
            obj(&[
                (
                    "listed_configurations",
                    Value::string("ALL_DECLARED_CHECKS_EXECUTED"),
                ),
                (
                    "configuration_completeness",
                    Value::string("USER_DECLARATION_NOT_EXHAUSTIVE"),
                ),
                (
                    "declared_inputs",
                    Value::string("GIT_AUTHENTICATED_NOT_COMPILER_MEMBERSHIP"),
                ),
                ("dependency_closure", Value::string("NOT_ESTABLISHED")),
            ]),
        ),
        (
            "execution_authority",
            Value::string("TRUSTED_LOCAL_NOT_ATTESTED"),
        ),
        (
            "artifact_build_provenance",
            Value::string("NOT_ESTABLISHED"),
        ),
        ("rea", Value::string("NOT_ESTABLISHED")),
    ])
    .stamp_id("report_id")
}

fn batch(source: &SourceRef, report: &Value) -> Result<Batch> {
    if report.get("schema")?.str()? != REPORT_SCHEMA
        || !report.content_id_matches("report_id")?
        || report.get("execution_authority")?.str()? != "TRUSTED_LOCAL_NOT_ATTESTED"
        || report.get("artifact_build_provenance")?.str()? != "NOT_ESTABLISHED"
        || report.get("rea")?.str()? != "NOT_ESTABLISHED"
    {
        return refuse("compiler matrix report corrupt or trust claims unsupported");
    }
    let receipt = Receipt {
        producer: "excession:compiler-matrix".to_owned(),
        version: "1".to_owned(),
        native_id: report.get("report_id")?.str()?.to_owned(),
        raw_json: report.canonical(),
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    Ok(Batch {
        source_commit: source.commit.clone(),
        receipts: vec![receipt],
        observations: vec![Observation {
            receipt_id,
            subject_id: Subject::Source(source.clone()).id()?,
            native_kind: "CONFIGURED_COMPILER_MATRIX".to_owned(),
            payload: obj(&[
                ("manifest", source_doc(source)),
                ("status", report.get("status")?.clone()),
                ("checks", report.get("checks")?.clone()),
                ("coverage", report.get("coverage")?.clone()),
                (
                    "execution_authority",
                    report.get("execution_authority")?.clone(),
                ),
                (
                    "artifact_build_provenance",
                    report.get("artifact_build_provenance")?.clone(),
                ),
                ("rea", report.get("rea")?.clone()),
            ]),
        }],
    })
}

fn snapshot(mut report: Value, id: &str) -> Result<Value> {
    report
        .entries_mut()?
        .insert("snapshot_id".into(), Value::string(id));
    Ok(report)
}

/// Execute each declared configuration with its native compiler, continuing
/// after diagnostic failures. Tool unavailability or unsafe input refuses the
/// matrix and retains any already-recorded individual child snapshots.
pub fn run(db: &Db, request: MatrixRequest<'_>) -> Result<Value> {
    if !request.trusted_local {
        return refuse("compiler matrix execution requires --trusted-local true");
    }
    let root = request
        .repo
        .canonicalize()
        .map_err(|e| format!("resolve compiler matrix checkout: {e}"))?;
    git::verify_repo(&root)?;
    git::clean(&root)?;
    let commit = git::rev(&root, request.revision)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("compiler matrix requires selected clean HEAD");
    }
    let (source, checks) = read_manifest(&root, &commit, request.manifest)?;
    live_file_matches(&root, &source)?;
    // No native tool executes until every tracked declaration is validated.
    for check in &checks {
        live_file_matches(&root, &check.configuration_source)?;
        for dependency in &check.declared_inputs {
            live_file_matches(&root, dependency)?;
        }
    }
    let mut results = Vec::new();
    for check in &checks {
        let executable = match check.language.as_str() {
            "rust" => request.rust_tool,
            "typescript" => request.typescript_tool,
            "python" => request.python_tool,
            _ => unreachable!("validated language"),
        };
        let receipt = compiler_check::run(
            db,
            Request {
                repo: &root,
                revision: &commit,
                language: &check.language,
                configuration: &check.configuration,
                source_prefix: check.source_prefix.as_deref(),
                executable,
                trusted_local: true,
            },
        )?;
        results.push(entry(check, &receipt)?);
    }
    git::clean(&root)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("compiler matrix changed Git HEAD");
    }
    live_file_matches(&root, &source)?;
    for check in &checks {
        for input in &check.declared_inputs {
            live_file_matches(&root, input)?;
        }
    }
    let report = build_report(&source, results)?;
    let id = evidence_store::persist(db, &batch(&source, &report)?)?;
    snapshot(report, &id)
}

/// Audit the original matrix and every child against pinned Git objects.
/// This does not turn self-reported local execution into independent attestation.
pub fn audit(db: &Db, repo: &Path, revision: &str, id: &str) -> Result<Value> {
    let original = evidence_store::audit(db, id)?;
    let rows = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(id)],
    )?;
    if rows.len() != 1 {
        return refuse("compiler matrix snapshot must contain exactly one receipt");
    }
    let receipt = json::parse(
        rows[0][0]
            .as_deref()
            .ok_or("missing compiler matrix receipt")?
            .as_bytes(),
    )?;
    if receipt.get("producer")?.str()? != "excession:compiler-matrix"
        || receipt.get("version")?.str()? != "1"
    {
        return refuse("not an original compiler matrix receipt");
    }
    let raw = receipt.get("raw_json")?.str()?;
    let claimed = json::parse(raw.as_bytes())?;
    if raw != claimed.canonical()
        || claimed.get("schema")?.str()? != REPORT_SCHEMA
        || !claimed.content_id_matches("report_id")?
        || receipt.get("native_id")?.str()? != claimed.get("report_id")?.str()?
    {
        return refuse("compiler matrix report is not its original canonical content");
    }
    let commit = git::rev(repo, revision)?;
    let path = claimed.get("manifest")?.get("path")?.str()?;
    let (source, checks) = read_manifest(repo, &commit, path)?;
    if claimed.get("source_commit")?.str()? != commit
        || claimed.get("source_tree")?.str()? != source.tree
        || claimed.get("manifest")? != &source_doc(&source)
    {
        return refuse("compiler matrix manifest differs from selected Git revision");
    }
    let stored_checks = claimed.get("checks")?.array_ref()?;
    if stored_checks.len() != checks.len() {
        return refuse("compiler matrix omitted a declared check");
    }
    let mut rebuilt = Vec::new();
    for (check, observed) in checks.iter().zip(stored_checks.iter()) {
        let child_id = observed.get("check_snapshot_id")?.str()?;
        let child = compiler_check::audit(db, repo, &commit, child_id)?;
        let expected = entry(check, &child)?;
        if observed != &expected {
            return refuse("compiler matrix check result differs from audited native receipt");
        }
        rebuilt.push(expected);
    }
    let result = build_report(&source, rebuilt)?;
    if claimed != result || evidence_store::manifest(&batch(&source, &result)?)? != original {
        return refuse("compiler matrix data or aggregate status differs from audited checks");
    }
    snapshot(result, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_manifest_and_schema_are_required() {
        let empty = json::parse(br#"{"schema":"excession.compiler-matrix-config.v1","checks":[]}"#)
            .unwrap();
        assert!(exact_fields(&empty, &["schema", "checks"]).is_ok());
        assert!(exact_fields(&empty, &["schema", "checks", "more"]).is_err());
    }

    #[test]
    fn a_single_failed_check_prevents_matrix_pass() {
        let source = SourceRef {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
            path: "checks.json".into(),
            blob: "c".repeat(40),
            mode: "100644".into(),
        };
        let report = build_report(
            &source,
            vec![
                obj(&[("status", Value::string("PASS"))]),
                obj(&[("status", Value::string("FAIL"))]),
            ],
        )
        .unwrap();
        assert_eq!(report.get("status").unwrap().str().unwrap(), "FAIL");
        assert_eq!(
            report
                .get("coverage")
                .unwrap()
                .get("dependency_closure")
                .unwrap()
                .str()
                .unwrap(),
            "NOT_ESTABLISHED"
        );
    }
}
