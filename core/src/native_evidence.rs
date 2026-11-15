//! Retain configured native Go/Clang/Java/Rust observations in the existing immutable
//! evidence store. All claimed Git inputs are replayed; producer semantics and
//! execution provenance remain explicitly unverified.
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, jvm_inputs, native_boundary, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_REPORT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INPUTS: usize = 512;

fn source_ref(repo: &Path, commit: &str, tree: &str, item: &Value) -> Result<SourceRef> {
    let path = item.get("path")?.str()?;
    git::validate_path(path)?;
    let expected = SourceRef::from_git(repo, commit, path)?;
    if item.get("source_commit")?.str()? != commit
        || item.get("source_tree")?.str()? != tree
        || item.get("blob")?.str()? != expected.blob
        || item.get("mode")?.str()? != expected.mode
    {
        return refuse("native source differs from immutable Git object identity");
    }
    Ok(expected)
}

fn source_value(input: &SourceRef) -> Value {
    obj(&[
        ("source_commit", Value::string(&input.commit)),
        ("source_tree", Value::string(&input.tree)),
        ("path", Value::string(&input.path)),
        ("blob", Value::string(&input.blob)),
        ("mode", Value::string(&input.mode)),
    ])
}

/// Validate a producer report and rebuild every immutable observation from
/// the original bytes. This is used both at ingestion and on each selection.
fn batch_from_bytes(repo: &Path, revision: &str, raw: &[u8]) -> Result<(Batch, Value)> {
    if raw.len() as u64 > MAX_REPORT_BYTES {
        return refuse("native evidence exceeds 4 MiB source receipt budget");
    }
    let report = json::parse(raw)?;
    if report.get("schema")?.str()? != "excession.native-analysis.v1"
        || report.get("status")?.str()? != "PARTIAL"
        || report.get("execution")?.str()? != "TRUSTED_LOCAL_NOT_ATTESTED"
        || !report.content_id_matches("report_id")?
    {
        return refuse("native evidence schema, status, or content identity mismatch");
    }
    let language = report.get("language")?.str()?;
    let producer = report.get("producer")?.str()?;
    let scope = report.get("scope")?.str()?;
    let valid = match language {
        "go" => producer == "go-list" && scope == "COMPILER_SELECTED_INPUTS_ONLY",
        "rust" => producer == "rust-analyzer-scip" && scope == "SCIP_WORKSPACE_REFERENCES_NOT_BUILD_VERIFIED",
        "java" => {
            (producer == "javac-symbols" && scope == "JAVAC_RESOLVED_SOURCE_REFERENCES_PARTIAL")
                || (producer == "javac-configured"
                    && scope == "JAVAC_CONFIGURED_SOURCE_REFERENCES_PARTIAL")
        }
        "c" | "cpp" => producer == "clang-ast" && scope == "CONFIGURED_TU_NOT_BUILD_VERIFIED",
        _ => false,
    };
    if !valid {
        return refuse("native producer, language and scope are inconsistent");
    }
    let commit = git::rev(repo, revision)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    if report.get("source_commit")?.str()? != commit || report.get("source_tree")?.str()? != tree {
        return refuse("native evidence belongs to a different Git revision");
    }
    let subject = report.get("source")?;
    let subject_ref = source_ref(repo, &commit, &tree, subject)?;
    let subject_path = subject_ref.path.as_str();
    let inputs = report.get("inputs")?.array_ref()?;
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return refuse("native evidence requires 1..=512 Git-bound inputs");
    }
    let nested = report.get("observation")?;
    let mut expected_roles = BTreeMap::new();
    if language == "go" {
        native_boundary::declaration_preflight(nested)?;
        if nested.get("language")?.str()? != "go" || nested.get("subject")?.str()? != subject_path {
            return refuse("Go input declaration subject differs from source");
        }
        for item in nested.get("inputs")?.array_ref()? {
            let path = item.get("path")?.str()?;
            let role = item.get("role")?.str()?;
            if expected_roles
                .insert(path.to_owned(), role.to_owned())
                .is_some()
            {
                return refuse("duplicate Go declared input");
            }
        }
    } else if language == "java" {
        if nested.get("schema")?.str()? != "excession.java-native.v1"
            || !nested.content_id_matches("report_id")?
            || nested.get("status")?.str()? != "JAVAC_ANALYZED_PROC_DISABLED"
            || nested.get("source_commit")?.str()? != commit
            || nested.get("source_tree")?.str()? != tree
            || nested.get("subject")?.str()? != subject_path
            || nested.get("configuration")?.str()?
                != if producer == "javac-configured" {
                    "EXTERNAL_RESOLVED_CLASSPATH_NO_PROCESSORS"
                } else {
                    "EMPTY_CLASSPATH_AND_SOURCEPATH_NO_PROCESSORS"
                }
            || nested.get("java_version")?.str()?.is_empty()
            || nested.get("java_version")?.str()?.len() > 100
        {
            return refuse("Java native observation lacks compiler and Git identity contract");
        }
    } else if language == "rust" {
        if nested.get("schema")?.str()? != "excession.scip-observations.v1"
            || !nested.content_id_matches("report_id")?
            || nested.get("source_commit")?.str()? != commit
            || nested.get("source_tree")?.str()? != tree
            || nested.get("indexer")?.get("name")?.str()? != "rust-analyzer"
            || nested.get("indexer")?.get("version")?.str()?.is_empty()
            || !nested.get("coverage")?.get("languages")?.array_ref()?.iter().any(|value| value.str().ok() == Some("rust"))
        {
            return refuse("Rust native index lacks a pinned rust-analyzer SCIP identity");
        }
    } else if nested.get("schema")?.str()? != "excession.clang-tu.v1"
        || !nested.content_id_matches("report_id")?
        || nested.get("source_commit")?.str()? != commit
        || nested.get("source_tree")?.str()? != tree
        || nested.get("observation")?.get("source_path")?.str()? != subject_path
        || nested.get("observation")?.get("source_blob")?.str()? != subject_ref.blob
        || nested.get("observation")?.get("language")?.str()?
            != if language == "c" { "c" } else { "c++" }
        || nested.get("observation")?.get("status")?.str()?
            != "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
    {
        return refuse("configured Clang observation differs from pinned source");
    }
    let mut validated = BTreeMap::new();
    let mut source_count = 0usize;
    let mut configuration_count = 0usize;
    let mut contains_subject = false;
    for input in inputs {
        let item = input.get("source")?;
        let git_source = source_ref(repo, &commit, &tree, item)?;
        let role = input.get("role")?.str()?;
        if !matches!(role, "source" | "configuration") {
            return refuse("native input role unsupported");
        }
        if language == "go"
            && expected_roles.get(&git_source.path).map(String::as_str) != Some(role)
        {
            return refuse("native Go input inventory differs from compiler declaration");
        }
        if git_source.path == subject_path {
            if role != "source" {
                return refuse("native subject must be a source input");
            }
            contains_subject = true;
        }
        if validated
            .insert(git_source.path.clone(), (git_source, role))
            .is_some()
        {
            return refuse("duplicate native evidence source input");
        }
        if role == "source" {
            source_count += 1;
        } else {
            configuration_count += 1;
        }
    }
    if !contains_subject
        || report.get("input_sources")?.integer()? != source_count as i64
        || report.get("input_configurations")?.integer()? != configuration_count as i64
        || (language == "go" && expected_roles.len() != validated.len())
        || (language == "java" && configuration_count != 0)
        || (language == "rust" && (source_count != 1 || configuration_count != 0 || validated.len() != 1))
        || (matches!(language, "c" | "cpp") && (source_count != 1 || configuration_count != 0))
    {
        return refuse("native evidence input counts or selected source differ");
    }
    let receipt = Receipt {
        producer: format!("native:{producer}"),
        version: "1".to_owned(),
        native_id: report.get("report_id")?.str()?.to_owned(),
        raw_json: std::str::from_utf8(raw)
            .map_err(|_| "native evidence must be UTF-8")?
            .to_owned(),
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    let mut observations = Vec::new();
    for (input, role) in validated.values() {
        observations.push(Observation {
            receipt_id: receipt_id.clone(),
            subject_id: Subject::Source(input.clone()).id()?,
            native_kind: "NATIVE_SELECTED_INPUT".to_owned(),
            payload: obj(&[
                ("source", source_value(input)),
                ("subject", subject.clone()),
                ("role", Value::string(*role)),
                ("scope", Value::string(scope)),
                ("producer", Value::string(producer)),
            ]),
        });
    }
    if language == "java" {
        let declared = nested.get("sources")?.array_ref()?;
        let mut source_paths = BTreeSet::new();
        for item in declared {
            let path = item.str()?;
            git::validate_path(path)?;
            if !source_paths.insert(path.to_owned()) {
                return refuse("duplicate Java declared source");
            }
        }
        if source_paths.len() != validated.len()
            || source_paths.iter().any(|p| !validated.contains_key(p))
        {
            return refuse("Java declaration differs from the authenticated source inventory");
        }
        if producer == "javac-configured" {
            let paths = source_paths.iter().cloned().collect::<Vec<_>>();
            jvm_inputs::verify(
                nested.get("resolved_inputs")?,
                &commit,
                &tree,
                nested.get("source_prefix")?.str()?,
                &paths,
            )?;
        } else if nested
            .entries()?
            .get("resolved_inputs")
            .is_some_and(|v| v != &Value::Null)
        {
            return refuse("unconfigured Java source report may not claim an external classpath");
        }
        if producer == "javac-configured" {
            let original = nested.get("resolved_inputs")?;
            observations.push(Observation {
                receipt_id: receipt_id.clone(),
                subject_id: Subject::Source(subject_ref.clone()).id()?,
                native_kind: "NATIVE_JVM_CLASSPATH_CONFIGURATION".to_owned(),
                payload: obj(&[
                    ("source", source_value(&subject_ref)),
                    ("subject", source_value(&subject_ref)),
                    ("scope", Value::string(scope)),
                    ("role", Value::string("external-compile-classpath")),
                    ("producer", Value::string(producer)),
                    ("provider_claim", original.get("provider_claim")?.clone()),
                    ("classpath", original.get("classpath")?.clone()),
                    ("release", original.get("release")?.clone()),
                    ("attestation", Value::string("NOT_ESTABLISHED")),
                ]),
            });
        }
        let references = nested.get("references")?.array_ref()?;
        if references.len() > 4096 {
            return refuse("Java resolved references exceed evidence budget");
        }
        let mut observed = BTreeSet::new();
        for reference in references {
            if reference.get("relation")?.str()? != "JAVAC_RESOLVED_SOURCE_REFERENCE" {
                return refuse("unknown Java compiler source-reference predicate");
            }
            let from = source_ref(repo, &commit, &tree, reference.get("from")?)?;
            let to = source_ref(repo, &commit, &tree, reference.get("to")?)?;
            if from.path == to.path
                || !source_paths.contains(&from.path)
                || !source_paths.contains(&to.path)
                || !observed.insert((from.path.clone(), to.path.clone()))
            {
                return refuse("duplicate, self, or untracked Java reference");
            }
            // The source is the referenced declaration: changing it can now
            // surface the observed dependent in impact without claiming a
            // cross-language build or runtime edge.
            observations.push(Observation {
                receipt_id: receipt_id.clone(),
                subject_id: Subject::Source(from.clone()).id()?,
                native_kind: "NATIVE_JAVA_SYMBOL_REFERENCE".to_owned(),
                payload: obj(&[
                    ("source", source_value(&to)),
                    ("subject", source_value(&from)),
                    ("from", source_value(&from)),
                    ("to", source_value(&to)),
                    ("role", Value::string("compiler-resolved-source-reference")),
                    ("scope", Value::string(scope)),
                    ("producer", Value::string(producer)),
                    ("relation", Value::string("JAVAC_RESOLVED_SOURCE_REFERENCE")),
                ]),
            });
        }
    }
    // SCIP occurrences retain SCIP semantics. Authenticate every observed
    // source against Git before admitting any edge; the requested Rust file
    // alone is not evidence of coverage for the whole indexed workspace.
    if language == "rust" {
        let references = nested.get("observations")?.array_ref()?;
        if references.len() > 4096 {
            return refuse("Rust SCIP observations exceed the native import relation budget");
        }
        let mut sources = BTreeMap::<String, SourceRef>::new();
        let mut seen = BTreeSet::new();
        for reference in references {
            if reference.get("relation")?.str()? != "SCIP_SYMBOL_REFERENCE" {
                return refuse("Rust native observation has a non-SCIP predicate");
            }
            let mut endpoints = Vec::with_capacity(2);
            for direction in ["from", "to"] {
                let item = reference.get(direction)?;
                let path = item.get("path")?.str()?;
                git::validate_path(path)?;
                if item.get("language")?.str()? != "rust" {
                    return refuse("Rust SCIP observation crosses an unsupported language boundary");
                }
                if !sources.contains_key(path) {
                    sources.insert(path.to_owned(), SourceRef::from_git(repo, &commit, path)?);
                }
                let source = &sources[path];
                if source.tree != tree || source.blob != item.get("blob")?.str()? {
                    return refuse("Rust SCIP endpoint differs from the pinned Git blob");
                }
                endpoints.push(source);
            }
            let symbol = reference.get("symbol")?.str()?;
            let line = reference.get("line")?.integer()?;
            if symbol.is_empty() || line < 1 || endpoints[0].path == endpoints[1].path {
                return refuse("Rust SCIP relation has invalid symbol, location or source pair");
            }
            if !seen.insert((
                endpoints[0].path.clone(),
                endpoints[1].path.clone(),
                symbol.to_owned(),
                line,
            )) {
                return refuse("duplicate Rust SCIP source reference");
            }
            observations.push(Observation {
                receipt_id: receipt_id.clone(),
                subject_id: Subject::Source(endpoints[0].clone()).id()?,
                native_kind: "SCIP_SYMBOL_REFERENCE".to_owned(),
                payload: reference.clone(),
            });
        }
        observations.push(Observation {
            receipt_id: receipt_id.clone(),
            subject_id: Subject::Source(subject_ref.clone()).id()?,
            native_kind: "NATIVE_RUST_WORKSPACE_INDEX".to_owned(),
            payload: obj(&[
                ("source", subject.clone()),
                ("subject", subject.clone()),
                ("scope", Value::string(scope)),
                ("producer", Value::string(producer)),
                ("role", Value::string("requested-source-not-index-coverage-proof")),
                ("indexed_documents", nested.get("coverage")?.get("indexed_documents")?.clone()),
                ("unresolved_references", nested.get("coverage")?.get("unresolved_references")?.clone()),
            ]),
        });
    }
    // Keep compiler AST symbols, calls, external headers and exact flags in the
    // original native receipt, not an invented universal source-level graph.
    if matches!(language, "c" | "cpp") {
        let ast = nested.get("observation")?;
        let symbols = ast.get("symbols")?.array_ref()?;
        let calls = ast.get("calls")?.array_ref()?;
        let headers = ast.get("dependencies")?.array_ref()?;
        if symbols.len() > 5000 || calls.len() > 5000 || headers.len() > 2048 {
            return refuse("Clang native observation exceeded safe import bounds");
        }
        // Promote only path-specific, hash-matching *tracked* header
        // observations. External/generated headers stay in the original
        // receipt without a fabricated Git identity or dependency edge.
        let mut observed_headers = BTreeSet::new();
        let mut blobs = git::BlobReader::open(repo)?;
        for header in headers {
            let path = match header.get("repo_path")? {
                Value::String(path) => path.as_str(),
                Value::Null => continue,
                _ => return refuse("native Clang header path must be a string or null"),
            };
            git::validate_path(path)?;
            if path == subject_path || !observed_headers.insert(path.to_owned()) {
                continue;
            }
            let sha = header.get("sha256")?.str()?;
            if sha.len() != 64
                || !sha
                    .bytes()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
            {
                return refuse("native Clang header has invalid content digest");
            }
            let reference = match SourceRef::from_git(repo, &commit, path) {
                Ok(reference) => reference,
                // A generated or untracked header cannot carry a Git source
                // identity, even if a native producer reported its contents.
                Err(_) => continue,
            };
            let contents = blobs.read_capped(&reference.blob, 4 * 1024 * 1024)?;
            if digest::sha256(&contents) != sha {
                return refuse("tracked Clang header content differs from Git blob");
            }
            observations.push(Observation {
                receipt_id: receipt_id.clone(),
                subject_id: Subject::Source(reference.clone()).id()?,
                native_kind: "NATIVE_CONFIGURED_HEADER".to_owned(),
                payload: obj(&[
                    ("source", source_value(&reference)),
                    ("subject", subject.clone()),
                    ("scope", Value::string(scope)),
                    ("producer", Value::string(producer)),
                    ("role", Value::string("header")),
                    ("sha256", Value::string(sha)),
                ]),
            });
        }
        observations.push(Observation {
            receipt_id: receipt_id.clone(),
            subject_id: Subject::Source(subject_ref).id()?,
            native_kind: "NATIVE_CONFIGURED_TU".to_owned(),
            payload: obj(&[
                ("source", subject.clone()),
                ("subject", subject.clone()),
                ("scope", Value::string(scope)),
                ("producer", Value::string(producer)),
                ("role", Value::string("configured-translation-unit")),
                ("symbol_count", Value::number(symbols.len())),
                ("call_count", Value::number(calls.len())),
                ("header_count", Value::number(headers.len())),
            ]),
        });
    }
    Ok((
        Batch {
            source_commit: commit,
            receipts: vec![receipt],
            observations,
        },
        report,
    ))
}

pub fn import(repo: &Path, revision: &str, report: &Path, db: &Db) -> Result<Value> {
    let size = std::fs::metadata(report)
        .map_err(|e| format!("native evidence stat: {e}"))?
        .len();
    if size > MAX_REPORT_BYTES {
        return refuse("native evidence input exceeds 4 MiB receipt budget");
    }
    let raw = std::fs::read(report).map_err(|e| format!("native evidence read: {e}"))?;
    let (batch, parsed) = batch_from_bytes(repo, revision, &raw)?;
    let id = evidence_store::persist(db, &batch)?;
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.native-evidence-import.v1"),
        ),
        ("snapshot_id", Value::string(id)),
        ("source_commit", parsed.get("source_commit")?.clone()),
        ("source_tree", parsed.get("source_tree")?.clone()),
        ("source", parsed.get("source")?.clone()),
        ("producer", parsed.get("producer")?.clone()),
        ("scope", parsed.get("scope")?.clone()),
        ("source_binding", Value::string("GIT_REPLAYED_AT_IMPORT")),
        ("semantics", Value::string("PRODUCER_SUPPLIED_UNATTESTED")),
    ]))
}

/// Rebuild from the exact stored producer bytes and compare the full immutable
/// snapshot, not just the report ID. Does not rerun Go or Clang.
pub fn audit_source(repo: &Path, revision: &str, db: &Db, snapshot: &str) -> Result<Value> {
    let persisted = evidence_store::audit(db, snapshot)?;
    let records = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot)],
    )?;
    if records.len() != 1 {
        return refuse("native snapshot requires one original producer receipt");
    }
    let receipt = json::parse(
        records[0][0]
            .as_deref()
            .ok_or("missing native receipt")?
            .as_bytes(),
    )?;
    let original = receipt.get("raw_json")?.str()?;
    let (batch, report) = batch_from_bytes(repo, revision, original.as_bytes())?;
    if receipt.get("producer")?.str()? != format!("native:{}", report.get("producer")?.str()?)
        || receipt.get("native_id")?.str()? != report.get("report_id")?.str()?
        || evidence_store::manifest(&batch)? != persisted
        || persisted.get("snapshot_id")?.str()? != snapshot
    {
        return refuse("stored native producer differs from Git-bound source replay");
    }
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.native-evidence-audit.v1"),
        ),
        (
            "status",
            Value::string("REPLAYED_GIT_BOUND_NATIVE_OBSERVATIONS"),
        ),
        ("snapshot_id", Value::string(snapshot)),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("source", report.get("source")?.clone()),
        ("producer", report.get("producer")?.clone()),
        ("scope", report.get("scope")?.clone()),
        ("producer_attestation", Value::string("NOT_ESTABLISHED")),
    ]))
}
