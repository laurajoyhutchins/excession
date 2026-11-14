//! Read-only adjudication of source-bound, published regression experiments.
//! A model prediction must already be frozen at A before C or migrated tests
//! enter the evaluation process. Oracle receipts are not producer-attested.
use crate::{Result, refuse};
use crate::git;
use crate::json::{self, obj, Value};
use std::path::Path;

const MAX_BYTES: u64 = 512 * 1024;
const MANIFEST_SCHEMA: &str =
    include_str!("../../contracts/concepts/regminer-cohort.v1.schema.json");
const ORACLE_SCHEMA: &str =
    include_str!("../../contracts/concepts/regminer-oracle.v1.schema.json");

fn read(path: &Path) -> Result<Value> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return refuse("regression evidence exceeds input bound");
    }
    json::parse(&std::fs::read(path).map_err(|e| e.to_string())?)
}
fn validate(value: &Value, text: &str) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("bad bundled schema: {e}"))?;
    let doc: serde_json::Value = serde_json::from_str(&value.canonical())
        .map_err(|e| e.to_string())?;
    let check = jsonschema::draft202012::new(&schema).map_err(|e| e.to_string())?;
    if let Err(e) = check.validate(&doc) {
        return refuse(format!("regression record violates contract: {e}"));
    }
    Ok(())
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> { v.get(key)?.str() }
fn oid_commit(repo: &Path, id: &str) -> Result<()> {
    git::oid(id)?;
    if git::rev(repo,id)? != id || git::text(repo,&["cat-file","-t",id])? != "commit" {
        return refuse("pinned source object is not an available commit");
    }
    Ok(())
}
fn chosen<'a>(manifest: &'a Value, name: &str) -> Result<&'a Value> {
    let mut found = None;
    for case in manifest.get("cases")?.array_ref()? {
        if string(case,"id")? == name {
            if found.is_some() { return refuse("ambiguous duplicate case ID"); }
            found = Some(case);
        }
    }
    found.ok_or_else(|| format!("unrecognized regression case {name}"))
}
fn has_file(repo: &Path, commit: &str, path: &str) -> Result<bool> {
    Ok(git::entries(repo,commit)?.iter().any(|e| e.path == path && e.kind == "blob"))
}
fn safe_ident(v: &str) -> Result<()> {
    if v.is_empty() || v.len() > 128 ||
        !v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.') {
        return refuse("invalid Java class or method identifier");
    }
    Ok(())
}

/// This function never executes a supplied script, shell command, Maven
/// project, dataset test, or inferred ontology rule.
pub fn adjudicate(
    repo: &Path, dataset: &Path, manifest_path: &Path, case_id: &str,
    frozen_path: &Path, oracle_path: &Path,
) -> Result<Value> {
    let manifest = read(manifest_path)?;
    let frozen = read(frozen_path)?;
    let oracle = read(oracle_path)?;
    validate(&manifest, MANIFEST_SCHEMA)?;
    validate(&oracle, ORACLE_SCHEMA)?;
    if string(&manifest,"dataset_repository")? != "SongXueZhi/RegMiner"
        || string(&manifest,"source_repository")? != "uniVocity/univocity-parsers" {
        return refuse("RegMiner demonstration cohort does not match expected projects");
    }
    git::verify_repo(repo)?;
    git::verify_repo(dataset)?;
    let dataset_revision = string(&manifest,"dataset_revision")?;
    oid_commit(dataset,dataset_revision)?;
    let doc = string(&manifest,"dataset_document")?;
    git::validate_path(doc)?;
    let observed_doc_blob = git::blob(dataset,dataset_revision,doc)?;
    if observed_doc_blob != string(&manifest,"dataset_document_blob")? {
        return refuse("RegMiner dataset documentation blob differs from source");
    }
    let case = chosen(&manifest,case_id)?;
    let a = string(case,"working")?;
    let b = string(case,"regression_inducing")?;
    let c = string(case,"repaired")?;
    for sha in [a,b,c] { oid_commit(repo,sha)?; }
    let parents = git::text(repo,&["rev-list","--parents","-n","1",b])?;
    let ids: Vec<_> = parents.split_whitespace().collect();
    if ids.len()!=2 || ids[0]!=b || ids[1]!=a {
        return refuse("RegMiner A is not the exact first parent of inducing B");
    }
    if b==c { return refuse("C must differ from B"); }
    git::text(repo,&["merge-base","--is-ancestor",b,c])?;
    let test = case.get("test")?;
    let test_path = string(test,"path")?;
    git::validate_path(test_path)?;
    let test_class = string(test,"class")?;
    let method = string(test,"method")?;
    safe_ident(test_class)?;
    safe_ident(method)?;
    let test_blob = git::blob(repo,c,test_path)?;
    let at_a = has_file(repo,a,test_path)?;
    let at_b = has_file(repo,b,test_path)?;
    if at_a || at_b {
        return refuse("expected a C-origin test requiring explicit migration to A/B");
    }
    let tree_a = git::rev(repo,&format!("{a}^{{tree}}"))?;
    let tree_b = git::rev(repo,&format!("{b}^{{tree}}"))?;
    let tree_c = git::rev(repo,&format!("{c}^{{tree}}"))?;
    if string(&frozen,"schema")? != "excession.concepts-temporal-obligations.v1"
        || string(&frozen,"mode")? != "PROSPECTIVE"
        || string(&frozen,"requested_commit")? != a
        || string(&frozen,"training_cutoff")? != a
        || string(&frozen,"prospective_commit")? != b
        || string(&frozen,"training_tree")? != tree_a
        || string(&frozen,"behavioral_validity")? != "NOT_EVALUATED"
        || frozen.get("safe_to_skip_unlisted_tests")?.bool()? {
        return refuse("prediction was not frozen at the required working revision");
    }
    if string(&oracle,"case_id")? != case_id
        || string(&oracle,"test_blob")? != test_blob {
        return refuse("migrated test or case identifier does not match the oracle");
    }
    let phase_values = oracle.get("phases")?.array_ref()?;
    if phase_values.len()!=3 { return refuse("expected exactly three oracle phases"); }
    let mut status = Vec::new();
    let mut observed_phases = Vec::new();
    for (i,(phase,sha,tree)) in
        [("A",a,&tree_a),("B",b,&tree_b),("C",c,&tree_c)].into_iter().enumerate() {
        let record=&phase_values[i];
        if string(record,"phase")?!=phase || string(record,"commit")?!=sha
            || string(record,"tree")?!=tree {
            return refuse("oracle phase ordering or Git identity mismatch");
        }
        let outcome = string(record,"status")?;
        let executions = record.get("tests_executed")?.integer()?;
        if (outcome=="UNSUPPORTED" && executions!=0)
            || (outcome!="UNSUPPORTED" && executions<1) {
            return refuse("reported test count conflicts with phase status");
        }
        status.push(outcome);
        observed_phases.push(record.clone());
    }
    let reproduced=status==["PASS","FAIL","PASS"];
    let proposals=frozen.get("policy_evaluation")?.get("predictions")?.array_ref()?;
    let visible: Vec<Value> = proposals.iter()
        .filter(|v| string(v,"commit").ok()==Some(b))
        .map(|v| obj(&[
            ("changed_source",v.get("changed_source").unwrap().clone()),
            ("model",v.get("selected_policy_prediction").unwrap().clone()),
            ("global_popularity",v.get("popularity_prediction").unwrap().clone()),
            ("same_directory",v.get("directory_prediction").unwrap().clone()),
        ])).collect();
    let mut report=obj(&[
        ("schema",Value::string("excession.concepts.regminer-adjudication.v1")),
        ("case_id",Value::string(case_id)),
        ("source_repository",manifest.get("source_repository")?.clone()),
        ("dataset_revision",Value::string(dataset_revision)),
        ("dataset_document_blob",Value::string(&observed_doc_blob)),
        ("manifest_sha256",Value::string(manifest.sha256())),
        ("frozen_sha256",Value::string(frozen.sha256())),
        ("oracle_sha256",Value::string(oracle.sha256())),
        ("working_commit",Value::string(a)),
        ("inducing_commit",Value::string(b)),
        ("repaired_commit",Value::string(c)),
        ("source_trees",obj(&[
            ("A",Value::string(&tree_a)),("B",Value::string(&tree_b)),("C",Value::string(&tree_c)),
        ])),
        ("migrated_test",obj(&[
            ("path",Value::string(test_path)),
            ("class",Value::string(test_class)),
            ("method",Value::string(method)),
            ("source_commit",Value::string(c)),
            ("source_blob",Value::string(&test_blob)),
            ("available_at_A",Value::Bool(at_a)),
            ("available_at_B",Value::Bool(at_b)),
        ])),
        ("oracle_phases",Value::array(observed_phases)),
        ("triple_recorded_from_supplied_oracle",Value::Bool(reproduced)),
        ("oracle_trust",Value::string("CALLER_SUPPLIED_UNATTESTED")),
        ("source_predictions",Value::array(visible)),
        ("historical_test_file_prediction_comparable",Value::Bool(false)),
        ("semantic_invariant_proven",Value::Bool(false)),
        ("better_than_native_selectors",Value::Bool(false)),
        ("status",Value::string(if reproduced {
            "MIGRATED_TRIPLE_RECORDED_NOT_ATTESTED"
        } else {"UNSUPPORTED_OR_INCONCLUSIVE"})),
    ]);
    let id=report.sha256();
    report.entries_mut()?.insert("report_id".into(),Value::string(id));
    Ok(report)
}
