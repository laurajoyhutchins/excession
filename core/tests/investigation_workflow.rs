//! Acceptance for investigation-driven source selection, synthesis and
//! conservative revision refresh. Does not certify program behavior.
use excession::sqlite::Db;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checked(cmd: &mut Command) -> Output {
    let output = cmd.output().expect("launch command");
    assert!(
        output.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, args: &[&str]) -> String {
    String::from_utf8(checked(Command::new("git").arg("-C").arg(root).args(args)).stdout)
        .unwrap().trim().to_owned()
}

fn commit(root: &Path, title: &str) -> String {
    git(root, &["add", "-A"]);
    checked(Command::new("git").arg("-C").arg(root).args([
        "-c", "user.name=Excession Test",
        "-c", "user.email=excession@example.invalid",
        "commit", "-qm", title,
    ]));
    git(root, &["rev-parse", "HEAD"])
}

fn setup() -> (Scratch, PathBuf, PathBuf, String) {
    let tmp = Scratch(std::env::temp_dir().join(format!(
        "excession-investigation-flow-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let repo = tmp.0.join("project");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("tests")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("src/recovery.rs"),
        "pub fn recover_worker() -> bool { retry_job(); true }\nfn retry_job() {}\n").unwrap();
    fs::write(repo.join("src/other.rs"), "pub fn display_canvas() {}\n").unwrap();
    fs::write(repo.join("tests/recovery_test.rs"), "#[test] fn recovery_test() { assert!(true); }\n").unwrap();
    let base = commit(&repo, "base");
    let db = tmp.0.join("ledger.sqlite");
    (tmp, repo, db, base)
}

fn command(args: &[&str]) -> Value {
    let output = checked(Command::new(env!("CARGO_BIN_EXE_excession")).args(args));
    serde_json::from_slice(&output.stdout).expect("canonical JSON")
}
fn refused(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(args).output().expect("run refusal");
    assert_eq!(output.status.code(), Some(2), "unexpected result: {output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("REFUSED:"));
}

fn schema(file: &str) -> serde_json::Value {
    serde_json::from_str(file).unwrap()
}

#[test]
fn questions_and_explanations_are_traceable_and_incomplete() {
    let (_tmp, repo, db, base) = setup();
    let repo = repo.to_str().unwrap();
    let db = db.to_str().unwrap();
    let started = command(&[
        "investigation", "start", "--repo", repo, "--db", db,
        "--rev", &base, "--question", "Where is worker recovery implemented?"
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let found = command(&[
        "investigation", "explore", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "1",
        "--limit", "8", "--index-segments", "256"
    ]);
    assert_eq!(found["kind"], "EXPLORE");
    assert_eq!(found["payload"]["report"]["schema"], "excession.agent-locate.v1");
    assert_eq!(found["payload"]["report"]["source_commit"], base);
    assert!(found["payload"]["report"].get("cache").is_none(),
        "volatile local cache path must not be in persistent report");
    assert_eq!(found["payload"]["report"]["safe_to_skip_unlisted_tests"], false);
    let reading = &found["payload"]["report"]["reading_plan"]["steps"];
    assert!(reading.is_array());
    assert!(reading.as_array().unwrap().iter().any(|step|
        step["source"]["path"] == "src/recovery.rs"
    ), "the actual recovery source should be in the inspection plan: {reading}");

    refused(&["investigation","explore","--repo",repo,"--db",db,
        "--id",id,"--expected-revision","1","--limit","8"]);
    let question = command(&[
        "investigation","question","--repo",repo,"--db",db,
        "--id",id,"--expected-revision","2",
        "--question","Does recovery retry after a persistence error?",
        "--reason","Current source search cannot establish runtime error conditions"
    ]);
    assert_eq!(question["kind"], "QUESTION");

    let claim = command(&[
        "investigation","claim","--repo",repo,"--db",db,
        "--id",id,"--expected-revision","3",
        "--path","src/recovery.rs",
        "--statement","recover_worker invokes retry_job",
        "--assumption","Reported from source inspection without execution",
        "--assessment","SOURCE_REVIEW_REPORTED"
    ]);
    let claim_id = claim["event_id"].as_str().unwrap();
    refused(&["investigation","synthesize","--repo",repo,"--db",db,
        "--id",id,"--expected-revision","4",
        "--narrative","The worker retries",
        "--claim-events",started["event_id"].as_str().unwrap()]);
    let synthesis = command(&[
        "investigation","synthesize","--repo",repo,"--db",db,
        "--id",id,"--expected-revision","4",
        "--narrative","Source inspection suggests the recovery entry point delegates to retry_job",
        "--claim-events",claim_id
    ]);
    assert_eq!(synthesis["kind"], "SYNTHESIS");
    assert_eq!(synthesis["payload"]["claim_event_ids"][0], claim_id);

    let explanation = command(&[
        "investigation","explain","--repo",repo,"--db",db,"--id",id
    ]);
    assert_eq!(explanation["schema"], "excession.investigation-explanation.v1");
    assert_eq!(explanation["revision"], 5);
    assert_eq!(explanation["implementation_map"][0]["path"], "src/recovery.rs");
    assert_eq!(explanation["claims"][0]["event_id"], claim_id);
    assert_eq!(explanation["narratives"][0]["claim_event_ids"][0], claim_id);
    assert_eq!(explanation["open_questions"][0]["state"], "OPEN");
    assert_eq!(explanation["retrievals"][0]["report_id"],
        found["payload"]["report"]["report_id"]);
    assert_eq!(explanation["completeness"], "INCOMPLETE_BEHAVIORAL_COVERAGE");
    assert_eq!(explanation["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(explanation["ready_to_merge"], false);

    let event_schema = schema(include_str!(
        "../../contracts/product/investigation-event.v1.schema.json"
    ));
    let view_schema = schema(include_str!(
        "../../contracts/product/investigation-explanation.v1.schema.json"
    ));
    let valid_event = jsonschema::draft202012::new(&event_schema).unwrap();
    let valid_view = jsonschema::draft202012::new(&view_schema).unwrap();
    let history = command(&["investigation","show","--repo",repo,"--db",db,"--id",id]);
    for event in history["events"].as_array().unwrap() {
        assert!(valid_event.is_valid(event), "event: {event}");
    }
    assert!(valid_view.is_valid(&explanation), "explanation: {explanation}");
    let mut false_proof = explanation.clone();
    false_proof["semantic_verification"] = json!("PROVEN");
    assert!(!valid_view.is_valid(&false_proof));
}

#[test]
fn refresh_distinguishes_direct_source_changes_from_unresolved_behavior() {
    let (_tmp, repo, db_path, base) = setup();
    let repo_str = repo.to_str().unwrap();
    let db_str = db_path.to_str().unwrap();
    let started = command(&[
        "investigation","start","--repo",repo_str,"--db",db_str,
        "--rev",&base,"--question","What must be preserved in recovery?"
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation","claim","--repo",repo_str,"--db",db_str,
        "--id",id,"--expected-revision","1",
        "--path","src/recovery.rs",
        "--statement","recovery invokes retry_job",
        "--assumption","source-level observation only",
        "--assessment","SOURCE_REVIEW_REPORTED"
    ]);
    let claim_id = claim["event_id"].as_str().unwrap();

    fs::write(repo.join("src/other.rs"), "pub fn display_canvas() { println!(\"ok\"); }\n").unwrap();
    let unrelated = commit(&repo, "unrelated change");
    let first = command(&[
        "investigation","refresh","--repo",repo_str,"--db",db_str,
        "--id",id,"--expected-revision","2","--candidate",&unrelated
    ]);
    assert_eq!(first["payload"]["claim_assessments"][0]["claim_event_id"], claim_id);
    assert_eq!(first["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_UNCHANGED");
    assert_eq!(first["payload"]["claim_assessments"][0]["semantic_impact"], "UNRESOLVED");
    assert_eq!(first["payload"]["dependency_completeness"], "NOT_ESTABLISHED");
    assert_eq!(first["payload"]["changed_path_count"], 1);

    fs::write(repo.join("src/recovery.rs"),
        "pub fn recover_worker() -> bool { false }\n").unwrap();
    let modified = commit(&repo, "changed recovery");
    let second = command(&[
        "investigation","refresh","--repo",repo_str,"--db",db_str,
        "--id",id,"--expected-revision","3","--candidate",&modified
    ]);
    assert_eq!(second["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_CHANGED");

    fs::remove_file(repo.join("src/recovery.rs")).unwrap();
    let deleted = commit(&repo, "deleted recovery");
    let third = command(&[
        "investigation","refresh","--repo",repo_str,"--db",db_str,
        "--id",id,"--expected-revision","4","--candidate",&deleted
    ]);
    assert_eq!(third["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_UNAVAILABLE");
    refused(&["investigation","refresh","--repo",repo_str,"--db",db_str,
        "--id",id,"--expected-revision","4","--candidate",&deleted]);

    let explanation = command(&[
        "investigation","explain","--repo",repo_str,"--db",db_str,"--id",id
    ]);
    assert_eq!(explanation["latest_refresh"]["candidate_commit"], deleted);
    assert_eq!(explanation["latest_refresh"]["semantic_verification"], "NOT_ESTABLISHED");

    let db = Db::open(&db_path).unwrap();
    assert!(db.execute(
        "DELETE FROM investigation_event_v1 WHERE investigation_id=?",
        &[excession::sqlite::Param::Text(id)]
    ).is_err());
}
