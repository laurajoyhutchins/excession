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
    let mut visible = Vec::new();
    for prediction in proposals {
        if string(prediction, "commit")? != b {
            continue;
        }
        visible.push(obj(&[
            ("changed_source", prediction.get("changed_source")?.clone()),
            ("model", prediction.get("selected_policy_prediction")?.clone()),
            ("global_popularity", prediction.get("popularity_prediction")?.clone()),
            ("same_directory", prediction.get("directory_prediction")?.clone()),
        ]));
    }
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn run(repo: &Path, args: &[&str]) -> String { git::text(repo,args).unwrap() }
    fn commit(repo: &Path, step: usize, path: &str) {
        let target=repo.join(path);
        if let Some(parent)=target.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(target,format!("revision {step}\n")).unwrap();
        run(repo,&["add","."]);
        run(repo,&["-c","user.name=Fixture","-c","user.email=fixture@example.invalid",
                   "commit","-qm","test fixture"]);
    }
    #[test]
    fn adjudicator_preserves_frozen_identity_and_never_promotes_unattested_oracles() {
        let root=std::env::temp_dir().join(format!("excession-regminer-{}-{}",
            std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir_all(&root).unwrap();
        run(&root,&["init","-q"]);
        fs::write(root.join("artifact-evaluation.md"),"published fixture\n").unwrap();
        commit(&root,0,"src/Main.java");
        for i in 1..=10 {
            commit(&root,i,if i%2==0 {"src/Main.java"} else {"src/Other.java"});
        }
        let a=run(&root,&["rev-parse","HEAD"]);
        commit(&root,11,"src/Main.java");
        let b=run(&root,&["rev-parse","HEAD"]);
        commit(&root,12,"src/test/java/CaseTest.java");
        let c=run(&root,&["rev-parse","HEAD"]);
        let blob=git::blob(&root,&c,"src/test/java/CaseTest.java").unwrap();
        let manifest=obj(&[
            ("schema",Value::string("excession.concepts.regminer-cohort.v1")),
            ("dataset_repository",Value::string("SongXueZhi/RegMiner")),
            ("dataset_revision",Value::string(&a)),
            ("dataset_document",Value::string("artifact-evaluation.md")),
            ("dataset_document_blob",Value::string(git::blob(&root,&a,"artifact-evaluation.md").unwrap())),
            ("source_repository",Value::string("uniVocity/univocity-parsers")),
            ("cases",Value::array([obj(&[
                ("id",Value::string("fixture")),
                ("working",Value::string(&a)),
                ("regression_inducing",Value::string(&b)),
                ("repaired",Value::string(&c)),
                ("test",obj(&[
                    ("path",Value::string("src/test/java/CaseTest.java")),
                    ("class",Value::string("CaseTest")),
                    ("method",Value::string("check")),
                ])),
            ])])),
        ]);
        let conf=crate::concepts_invariants::Settings {
            train:8,holdout:0,support:2,confidence_percent:60,prefix:".".into(),
        };
        let frozen=crate::concepts_invariants::propose(&root,&a,&conf,Some(&b)).unwrap();
        let oracle=|broken:bool| obj(&[
            ("schema",Value::string("excession.concepts.regminer-oracle.v1")),
            ("case_id",Value::string("fixture")),
            ("test_blob",Value::string(&blob)),
            ("phases",Value::array(["A","B","C"].iter().enumerate().map(|(idx,phase)| {
                let commit=[&a,&b,&c][idx];
                obj(&[
                    ("phase",Value::string(*phase)),
                    ("commit",Value::string(commit)),
                    ("tree",Value::string(git::rev(&root,&format!("{commit}^{{tree}}")).unwrap())),
                    ("status",Value::string(if idx==1 && !broken {"FAIL"} else {"PASS"})),
                    ("tests_executed",Value::number(1)),
                    ("log_sha256",Value::string("0".repeat(64))),
                ])
            }))),
        ]);
        let manifest_path=root.join("manifest.json");
        let freeze_path=root.join("frozen.json");
        let oracle_path=root.join("oracle.json");
        fs::write(&manifest_path,manifest.canonical()).unwrap();
        fs::write(&freeze_path,frozen.canonical()).unwrap();
        fs::write(&oracle_path,oracle(false).canonical()).unwrap();
        let report=adjudicate(&root,&root,&manifest_path,"fixture",
            &freeze_path,&oracle_path).unwrap();
        assert!(report.get("triple_recorded_from_supplied_oracle").unwrap().bool().unwrap());
        assert_eq!(report.get("oracle_trust").unwrap().str().unwrap(),"CALLER_SUPPLIED_UNATTESTED");
        assert!(!report.get("historical_test_file_prediction_comparable").unwrap().bool().unwrap());
        fs::write(&oracle_path,oracle(true).canonical()).unwrap();
        let negative=adjudicate(&root,&root,&manifest_path,"fixture",
            &freeze_path,&oracle_path).unwrap();
        assert!(!negative.get("triple_recorded_from_supplied_oracle").unwrap().bool().unwrap());
        let mut corrupted=frozen.clone();
        corrupted.entries_mut().unwrap().insert("training_cutoff".into(),Value::string(&b));
        fs::write(&freeze_path,corrupted.canonical()).unwrap();
        assert!(adjudicate(&root,&root,&manifest_path,"fixture",
            &freeze_path,&oracle_path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}


/// Import a published BugsInPy B/C pair from exact Git blob metadata.
/// BugsInPy does not supply a proven pre-regression A revision. This command
/// records that gap explicitly and NEVER executes dataset shell scripts.
pub fn bugsinpy_import(
    dataset: &Path, revision: &str, project: &str, bug_id: &str,
) -> Result<Value> {
    git::verify_repo(dataset)?;
    oid_commit(dataset,revision)?;
    if project.is_empty() || project.len() > 64
        || !project.bytes().all(|c| c.is_ascii_alphanumeric() || c==b'-' || c==b'_')
        || bug_id.parse::<u16>().ok().filter(|v| *v > 0).is_none() {
        return refuse("invalid bounded BugsInPy project or bug identifier");
    }
    let project_path=format!("projects/{project}/project.info");
    let info_path=format!("projects/{project}/bugs/{bug_id}/bug.info");
    let test_path=format!("projects/{project}/bugs/{bug_id}/run_test.sh");
    let mut reader=git::BlobReader::open(dataset)?;
    let project_blob=git::blob(dataset,revision,&project_path)?;
    let info_blob=git::blob(dataset,revision,&info_path)?;
    let test_blob=git::blob(dataset,revision,&test_path)?;
    let project_info=reader.read_capped(&project_blob,4096)?;
    let bug_info=reader.read_capped(&info_blob,4096)?;
    reader.finish()?;
    let parse=|bytes: &[u8]| -> Result<std::collections::BTreeMap<String,String>> {
        let text=std::str::from_utf8(bytes).map_err(|_| "non-UTF8 BugsInPy metadata")?;
        let mut fields=std::collections::BTreeMap::new();
        for line in text.lines() {
            let line=line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let (key,raw)=line.split_once('=').ok_or("invalid BugsInPy metadata assignment")?;
            if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphabetic() || b==b'_')
                || !raw.starts_with('"') || !raw.ends_with('"') || raw.len()<2
                || raw[1..raw.len()-1].contains('"') {
                return refuse("unsupported BugsInPy metadata syntax");
            }
            if fields.insert(key.to_owned(),raw[1..raw.len()-1].to_owned()).is_some() {
                return refuse("duplicate BugsInPy metadata key");
            }
        }
        Ok(fields)
    };
    let project_info=parse(&project_info)?;
    let bug_info=parse(&bug_info)?;
    let get=|key: &str| -> Result<&str> {
        bug_info.get(key).map(String::as_str)
            .ok_or_else(|| format!("BugsInPy bug missing {key}"))
    };
    let buggy=get("buggy_commit_id")?;
    let fixed=get("fixed_commit_id")?;
    git::oid(buggy)?;
    git::oid(fixed)?;
    if buggy==fixed { return refuse("BugsInPy buggy and fixed commits must differ"); }
    let url=project_info.get("github_url").ok_or("BugsInPy project lacks source repository URL")?;
    if !url.starts_with("https://github.com/") || url.len() > 256
        || url.chars().any(|c| c.is_ascii_control() || c.is_whitespace()) {
        return refuse("BugsInPy upstream URL is not a supported GitHub URL");
    }
    let case=format!("{project}-{bug_id}");
    let mut report=obj(&[
        ("schema",Value::string("excession.concepts.bugsinpy-import.v1")),
        ("status",Value::string("B_C_METADATA_ONLY_A_NOT_ESTABLISHED")),
        ("dataset_repository",Value::string("soarsmu/BugsInPy")),
        ("dataset_revision",Value::string(revision)),
        ("case",Value::string(case)),
        ("source_repository_url",Value::string(url)),
        ("working_commit_A",Value::Null),
        ("buggy_commit_B",Value::string(buggy)),
        ("fixed_commit_C",Value::string(fixed)),
        ("test_files",Value::string(get("test_file")?)),
        ("python_version",Value::string(get("python_version")?)),
        ("dataset_project_info_blob",Value::string(project_blob)),
        ("dataset_bug_info_blob",Value::string(info_blob)),
        ("dataset_test_script_blob",Value::string(test_blob)),
        ("script_executed",Value::Bool(false)),
        ("source_commits_verified",Value::Bool(false)),
        ("behavioral_oracle_executed",Value::Bool(false)),
        ("semantic_invariant_proven",Value::Bool(false)),
    ]);
    let id=report.sha256();
    report.entries_mut()?.insert("report_id".into(),Value::string(id));
    Ok(report)
}
