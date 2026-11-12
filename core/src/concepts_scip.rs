//! Concept observations from already Git-bound, officially decoded SCIP indexes.
//!
//! Reuse scip_refactor's single native-index/Git validation. These are symbol
//! references, not imports, calls, execution witnesses or semantic equivalence.
use crate::concepts;
use crate::concepts_normalize;
use crate::json::{Value, obj};
use crate::scip_refactor;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_REFERENCE_BATCH: usize = 10_000;
const PRODUCER: &str = "excession.scip-references.v1";

fn subject(commit: &str, receipt: &str, source: &Value, line: Option<i64>) -> Result<Value> {
    let mut result = obj(&[
        ("kind", Value::string("SOURCE_FILE")),
        ("source_commit", Value::string(commit)),
        ("path", source.get("path")?.clone()),
        ("blob", source.get("blob")?.clone()),
        ("producer", Value::string(PRODUCER)),
        ("producer_receipt_id", Value::string(receipt)),
    ]);
    if let Some(line) = line {
        if line < 1 {
            return refuse("SCIP reference must identify a positive source line");
        }
        result.entries_mut()?.insert(
            "span".into(),
            obj(&[
                ("start_line", Value::number(line)),
                ("end_line", Value::number(line)),
                ("encoding", Value::string("LINE_ONLY")),
            ]),
        );
    }
    concepts::seal(result, "subject_id")
}

/// Normalize the complete matched cross-file reference subset with honest
/// unknown coverage. Source matching stays inside scip_refactor's validated
/// native scan. No independent invocation of the configured SCIP indexer.
pub fn normalize_index(
    repo: &Path,
    revision: &str,
    index: &Path,
    source_prefix: Option<&str>,
) -> Result<Value> {
    let native = scip_refactor::references_with_prefix(repo, revision, index, source_prefix)?;
    normalize_report(&native)
}

/// Convert the complete Git-checked native reference report, retaining
/// each upstream symbol and its exact referenced source evidence.
fn normalize_report(native: &Value) -> Result<Value> {
    if concepts::content_id(native, "report_id")? != native.get("report_id")?.str()? {
        return refuse("SCIP report identity does not match content");
    }
    if native.get("schema")?.str()? != "excession.scip-references.v1"
        || native.get("status")?.str()? != "GIT_BOUND_INDEX_NOT_REEXECUTED"
    {
        return refuse("SCIP reference report lacks validated source provenance");
    }
    let commit = native.get("source_commit")?.str()?;
    let tree = native.get("source_tree")?.str()?;
    let receipt = native.get("report_id")?.str()?;
    let refs = native.get("references")?.array_ref()?;
    if refs.len() > MAX_REFERENCE_BATCH {
        return refuse("SCIP concept reference batch exceeds 10000 observations");
    }
    let mut docs = BTreeMap::new();
    for source in native.get("documents")?.array_ref()? {
        let path = source.get("path")?.str()?;
        crate::git::validate_path(path)?;
        if docs.insert(path.to_owned(), source.clone()).is_some() {
            return refuse("duplicate validated SCIP source document");
        }
    }
    let mut scopes = BTreeSet::new();
    for path in docs.keys() {
        scopes.insert(path.clone());
    }
    let mut subjects: BTreeMap<String, Value> = BTreeMap::new();
    let mut observations = Vec::new();
    let mut source_refs = BTreeSet::new();
    for reference in refs {
        if reference.get("relation")?.str()? != "SCIP_SYMBOL_REFERENCE" {
            return refuse("SCIP native reference report contains a non-SCIP relation");
        }
        let from = reference.get("from")?;
        let to = reference.get("to")?;
        let from_path = from.get("path")?.str()?;
        let to_path = to.get("path")?.str()?;
        if from_path == to_path {
            return refuse("SCIP cross-file reference unexpectedly points to itself");
        }
        for side in [from, to] {
            let name = side.get("path")?.str()?;
            let stored = docs
                .get(name)
                .ok_or("reference lacks indexed source document")?;
            if side != stored {
                return refuse("SCIP reference source binding differs from indexed document");
            }
        }
        let specifier = reference.get("symbol")?.str()?;
        if specifier.is_empty() || specifier.len() > 4096 || specifier.starts_with("local ") {
            return refuse("SCIP symbol reference must be a bounded global symbol identity");
        }
        let line = reference.get("line")?.integer()?;
        let origin = subject(commit, receipt, from, Some(line))?;
        let destination = subject(commit, receipt, to, None)?;
        let origin_id = origin.get("subject_id")?.str()?.to_owned();
        let destination_id = destination.get("subject_id")?.str()?.to_owned();
        subjects.insert(origin_id.clone(), origin);
        subjects.insert(destination_id.clone(), destination);
        let source_observation_id = reference.sha256();
        if !source_refs.insert(source_observation_id.clone()) {
            return refuse("duplicate native SCIP reference");
        }
        let record = concepts::seal(
            obj(&[
                ("receipt_id", Value::string(receipt)),
                (
                    "source_observation_id",
                    Value::string(source_observation_id),
                ),
                ("native_relation", Value::string("SCIP_SYMBOL_REFERENCE")),
                ("kind", Value::string("SCIP_REFERENCE")),
                ("status", Value::string("OBSERVED")),
                ("source_subject_id", Value::string(origin_id)),
                ("target_subject_id", Value::string(destination_id)),
                ("language", from.get("language")?.clone()),
                ("specifier", Value::string(specifier)),
                ("condition_evaluation", Value::string("NOT_EVALUATED")),
                ("rule", Value::string("SCIP_UNIQUE_INDEXED_DEFINITION")),
            ]),
            "observation_id",
        )?;
        observations.push(record);
    }
    observations.sort_by_key(Value::canonical);
    let coverage = native.get("coverage")?;
    let unresolved = coverage.get("unresolved_references")?.integer()?;
    let mut gaps = Vec::new();
    if unresolved > 0 {
        gaps.push(obj(&[
            ("reason", Value::string("UNRESOLVED_SYMBOL")),
            (
                "native_reason",
                Value::string(format!(
                    "SCIP references without unique indexed definition: {unresolved}"
                )),
            ),
        ]));
    }
    // Definition completeness, native indexer execution and dynamic bindings
    // are not established by this data, even when all indexed references match.
    gaps.push(obj(&[
        ("reason", Value::string("PARTIAL_CONFIGURATION")),
        (
            "native_reason",
            Value::string("SCIP_INDEX_SCOPE_NOT_PROVEN_COMPLETE"),
        ),
    ]));
    let mut batch = obj(&[
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
                ("producer", native.get("indexer")?.get("name")?.clone()),
                ("version", native.get("indexer")?.get("version")?.clone()),
                ("kind", Value::string("SCIP")),
                ("configuration_id", native.get("index_sha256")?.clone()),
                ("scope", Value::array(scopes.into_iter().map(Value::string))),
            ])]),
        ),
        ("observations", Value::array(observations)),
        ("gaps", Value::array(gaps)),
    ]);
    batch = concepts::seal(batch, "batch_id")?;
    concepts_normalize::validate_batch(&batch)?;
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_a_self_inconsistent_native_receipt_without_git_access() {
        let forged = obj(&[
            ("schema", Value::string("excession.scip-references.v1")),
            ("status", Value::string("GIT_BOUND_INDEX_NOT_REEXECUTED")),
            ("source_commit", Value::string("a".repeat(40))),
            ("source_tree", Value::string("b".repeat(40))),
            ("report_id", Value::string("c".repeat(64))),
            ("references", Value::array(vec![])),
        ]);
        assert!(normalize_report(&forged).is_err());
    }
}
