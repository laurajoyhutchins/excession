//! Normalize candidate-only semantic receipts without changing their native meaning.
//! A source-bound import candidate is not a call, build input, or runtime edge.
use crate::concepts;
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const LEGACY_SCHEMA: &str =
    include_str!("../../contracts/semantic/semantic-observations.v1.schema.json");
const BATCH_SCHEMA: &str =
    include_str!("../../contracts/concepts/observation-batch.v1.schema.json");

fn schema_check(value: &Value, schema_text: &str) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(schema_text).map_err(|e| format!("concept schema: {e}"))?;
    let instance: serde_json::Value =
        serde_json::from_str(&value.canonical()).map_err(|e| format!("concept input: {e}"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("compile concepts schema: {e}"))?;
    if let Err(e) = validator.validate(&instance) {
        return refuse(format!("semantic normalization contract violation: {e}"));
    }
    Ok(())
}

fn source_subject(commit: &str, receipt: &str, source: &Value, line: bool) -> Result<Value> {
    let mut fields = obj(&[
        ("kind", Value::string("SOURCE_FILE")),
        ("source_commit", Value::string(commit)),
        ("path", source.get("path")?.clone()),
        ("blob", source.get("blob")?.clone()),
        (
            "producer",
            Value::string("excession.semantic-observations.v1"),
        ),
        ("producer_receipt_id", Value::string(receipt)),
    ]);
    if line {
        fields.entries_mut()?.insert(
            "span".into(),
            obj(&[
                ("start_line", source.get("line")?.clone()),
                ("end_line", source.get("line")?.clone()),
                ("encoding", Value::string("LINE_ONLY")),
            ]),
        );
    }
    concepts::seal(fields, "subject_id")
}

/// Check an existing static-candidate report against the exact Git tree and
/// preserve the original observation identifiers, kinds and coverage gaps.
pub fn normalize_static(repo: &Path, source_report: &Value) -> Result<Value> {
    schema_check(source_report, LEGACY_SCHEMA)?;
    if concepts::content_id(source_report, "report_id")? != source_report.get("report_id")?.str()? {
        return refuse("semantic report ID does not match canonical content");
    }
    let commit = source_report.get("source_commit")?.str()?;
    let tree = source_report.get("source_tree")?.str()?;
    git::verify_repo(repo)?;
    if git::rev(repo, &format!("{commit}^{{tree}}"))? != tree {
        return refuse("semantic source tree does not match pinned commit");
    }
    let entries = git::entries(repo, commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob" && matches!(entry.mode.as_str(), "100644" | "100755"))
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let tracked = |source: &Value| -> Result<()> {
        let path = source.get("path")?.str()?;
        let blob = source.get("blob")?.str()?;
        git::validate_path(path)?;
        if entries.get(path).map(|entry| entry.oid.as_str()) != Some(blob) {
            return refuse(format!(
                "semantic observation not bound to Git blob: {path}"
            ));
        }
        Ok(())
    };
    let record_list = source_report.get("records")?.array_ref()?;
    let gaps = source_report.get("gaps")?.array_ref()?;
    if record_list.len() as i64 != source_report.get("record_count")?.integer()?
        || gaps.len() as i64 != source_report.get("gap_count")?.integer()?
    {
        return refuse("semantic report counts do not match records or gaps");
    }
    let receipt = source_report.get("report_id")?.str()?;
    let mut subjects: BTreeMap<String, Value> = BTreeMap::new();
    let mut observations = Vec::new();
    let mut scopes = BTreeSet::new();
    let mut observed = BTreeSet::new();
    for item in record_list {
        if concepts::content_id(item, "observation_id")? != item.get("observation_id")?.str()? {
            return refuse("semantic observation ID differs from canonical record");
        }
        if item.get("kind")?.str()? != "IMPORTS" {
            return refuse("semantic observation kind unsupported by candidate normalizer");
        }
        let status = item.get("assessment")?.get("status")?.str()?;
        if !matches!(status, "CANDIDATE" | "UNRESOLVED") {
            return refuse("unsupported semantic candidate status");
        }
        let source = item.get("subject")?;
        tracked(source)?;
        let src = source_subject(commit, receipt, source, true)?;
        let src_id = src.get("subject_id")?.str()?.to_owned();
        scopes.insert(source.get("path")?.str()?.to_owned());
        subjects.insert(src_id.clone(), src);
        let mut record = obj(&[
            ("receipt_id", Value::string(receipt)),
            ("source_observation_id", item.get("observation_id")?.clone()),
            ("native_relation", Value::string("IMPORTS")),
            ("kind", Value::string("STATIC_IMPORT_CANDIDATE")),
            ("status", Value::string(status)),
            ("source_subject_id", Value::string(src_id)),
            ("language", source.get("language")?.clone()),
            ("specifier", item.get("specifier")?.clone()),
            (
                "condition_evaluation",
                item.get("condition_evaluation")?.clone(),
            ),
            ("rule", item.get("assessment")?.get("rule")?.clone()),
        ]);
        if status == "CANDIDATE" {
            let target = item.get("target")?;
            tracked(target)?;
            let dst = source_subject(commit, receipt, target, false)?;
            let dst_id = dst.get("subject_id")?.str()?.to_owned();
            subjects.insert(dst_id.clone(), dst);
            record
                .entries_mut()?
                .insert("target_subject_id".into(), Value::string(dst_id));
        }
        let record = concepts::seal(record, "observation_id")?;
        if !observed.insert(record.get("source_observation_id")?.str()?.to_owned()) {
            return refuse("duplicate native semantic observation");
        }
        observations.push(record);
    }
    observations.sort_by_key(Value::canonical);
    let mut translated_gaps = Vec::new();
    for gap in gaps {
        let original = gap.get("status")?.str()?;
        let reason = match original {
            "RESOURCE_LIMIT" => "RESOURCE_LIMIT",
            "UNSUPPORTED_LANGUAGE" | "NO_RESOLVER_FOR_LANGUAGE" => "UNSUPPORTED_SOURCE",
            "GRAMMAR_NOT_INSTALLED" => "MISSING_PRODUCER",
            _ => "PARTIAL_CONFIGURATION",
        };
        let path = gap.get("source_path")?.str()?;
        git::validate_path(path)?;
        translated_gaps.push(obj(&[
            ("source_path", Value::string(path)),
            ("reason", Value::string(reason)),
            ("native_reason", Value::string(original)),
        ]));
    }
    translated_gaps.sort_by_key(Value::canonical);
    let batch = obj(&[
        (
            "schema",
            Value::string("excession.concepts-observation-batch.v1"),
        ),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("completeness", Value::string("PARTIAL_CANDIDATES_ONLY")),
        ("subjects", Value::array(subjects.into_values())),
        (
            "inputs",
            Value::array(vec![obj(&[
                ("receipt_id", Value::string(receipt)),
                ("producer", Value::string("excession.semantic-observations")),
                ("version", Value::string("1")),
                ("kind", Value::string("STATIC_SYNTAX")),
                (
                    "configuration_id",
                    source_report.get("syntax_report_id")?.clone(),
                ),
                ("scope", Value::array(scopes.into_iter().map(Value::string))),
            ])]),
        ),
        ("observations", Value::array(observations)),
        ("gaps", Value::array(translated_gaps)),
    ]);
    let result = concepts::seal(batch, "batch_id")?;
    validate_batch(&result)?;
    Ok(result)
}

pub fn validate_batch(batch: &Value) -> Result<()> {
    schema_check(batch, BATCH_SCHEMA)?;
    if concepts::content_id(batch, "batch_id")? != batch.get("batch_id")?.str()? {
        return refuse("noncanonical normalized batch ID");
    }
    let mut subjects = BTreeSet::new();
    for subject in batch.get("subjects")?.array_ref()? {
        if concepts::content_id(subject, "subject_id")? != subject.get("subject_id")?.str()? {
            return refuse("noncanonical normalized subject ID");
        }
        if subject.get("source_commit")?.str()? != batch.get("source_commit")?.str()? {
            return refuse("cross-revision normalized subject");
        }
        if !subjects.insert(subject.get("subject_id")?.str()?.to_owned()) {
            return refuse("duplicate normalized subject");
        }
    }
    let mut receipts = BTreeSet::new();
    for input in batch.get("inputs")?.array_ref()? {
        if !receipts.insert(input.get("receipt_id")?.str()?.to_owned()) {
            return refuse("duplicate normalized input receipt");
        }
    }
    let mut ids = BTreeSet::new();
    for item in batch.get("observations")?.array_ref()? {
        if concepts::content_id(item, "observation_id")? != item.get("observation_id")?.str()? {
            return refuse("noncanonical normalized observation");
        }
        if !ids.insert(item.get("observation_id")?.str()?.to_owned()) {
            return refuse("duplicate normalized observation");
        }
        if !subjects.contains(item.get("source_subject_id")?.str()?)
            || !receipts.contains(item.get("receipt_id")?.str()?)
        {
            return refuse("normalized evidence references absent source or receipt");
        }
        match item.get("status")?.str()? {
            "CANDIDATE" if !subjects.contains(item.get("target_subject_id")?.str()?) => {
                return refuse("candidate target missing from normalized subjects");
            }
            "UNRESOLVED" if item.get("target_subject_id").is_ok() => {
                return refuse("unresolved candidate invented a target");
            }
            _ => (),
        }
    }
    Ok(())
}

/// Strict bounded JSON loader for a saved, explicit semantic observation report.
pub fn normalize_file(repo: &Path, path: &Path) -> Result<Value> {
    let stat = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if stat.len() > 32 * 1024 * 1024 {
        return refuse("semantic observation report exceeds 32 MiB");
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    normalize_static(repo, &json::parse(&bytes)?)
}
