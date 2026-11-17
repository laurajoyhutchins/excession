//! Real Git/SQLite acceptance for question-led investigation persistence.
//! All observations remain advisory; this test cannot certify target behavior.
use excession::investigation;
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

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("launch native command");
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git(root: &Path, args: &[&str]) -> String {
    String::from_utf8(checked(Command::new("git").arg("-C").arg(root).args(args)).stdout)
        .unwrap().trim().to_owned()
}

fn commit(root: &Path, title: &str) -> String {
    git(root, &["add", "."]);
    checked(Command::new("git").arg("-C").arg(root).args([
        "-c", "user.name=Excession Acceptance",
        "-c", "user.email=excession@example.invalid",
        "commit", "-qm", title,
    ]));
    git(root, &["rev-parse", "HEAD"])
}

fn repo() -> (Scratch, PathBuf, PathBuf, String) {
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "excession-investigation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    let source = scratch.0.join("project");
    fs::create_dir_all(source.join("src")).unwrap();
    git(&source, &["init", "-q"]);
    fs::write(source.join("src/worker.rs"), "pub fn recover() -> bool { true }\n").unwrap();
    let commit = commit(&source, "base");
    let database = scratch.0.join("investigations.sqlite");
    (scratch, source, database, commit)
}

fn cli(args: &[&str]) -> Value {
    let output = checked(Command::new(env!("CARGO_BIN_EXE_excession")).args(args));
    serde_json::from_slice(&output.stdout).expect("canonical JSON")
}

fn refused(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(args).output().expect("run CLI refusal");
    assert_eq!(output.status.code(), Some(2), "unexpected success: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("REFUSED:"),
        "{output:?}"
    );
}

#[test]
fn persistent_investigation_connects_source_claim_plan_and_agent_verification() {
    let (_tmp, repo, db, base) = repo();
    let repo = repo.to_str().unwrap();
    let db = db.to_str().unwrap();
    let question = "How does the worker recover and which behavior must survive extraction?";

    let created = cli(&["investigation", "start", "--repo", repo, "--db", db,
        "--rev", &base, "--question", question]);
    assert_eq!(created["schema"], "excession.investigation-event.v1");
    assert_eq!(created["kind"], "CREATE");
    assert_eq!(created["revision"], 1);
    let id = created["investigation_id"].as_str().unwrap();
    let created_again = investigation::investigation_id(
        &base,
        created["source_tree"].as_str().unwrap(),
        question,
    ).unwrap();
    assert_eq!(id, created_again);

    let claim = cli(&["investigation", "claim", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "1", "--path", "src/worker.rs",
        "--statement", "recover currently returns true",
        "--assumption", "source has been inspected; no runtime execution observed",
        "--assessment", "SOURCE_REVIEW_REPORTED"]);
    assert_eq!(claim["kind"], "CLAIM");
    assert_eq!(claim["payload"]["source"]["commit"], base);
    assert_eq!(claim["payload"]["assessment"], "SOURCE_REVIEW_REPORTED");
    let claim_event = claim["event_id"].as_str().unwrap();

    refused(&["investigation", "claim", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "1", "--path", "src/worker.rs",
        "--statement", "stale update", "--assumption", "read source",
        "--assessment", "HYPOTHESIS"]);
    refused(&["investigation", "claim", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "2", "--path", "../secret",
        "--statement", "invalid source", "--assumption", "read source",
        "--assessment", "HYPOTHESIS"]);
    refused(&["investigation", "claim", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "2", "--path", "src/worker.rs",
        "--statement", "not observed", "--assumption", "read source",
        "--assessment", "OBSERVED"]);

    let plan = cli(&["investigation", "plan", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "2",
        "--claim-event", claim_event,
        "--obligation", "preserve recover() returning true for all existing callers"]);
    assert_eq!(plan["kind"], "PLAN");
    assert_eq!(plan["payload"]["claim_event_id"], claim_event);
    let plan_event = plan["event_id"].as_str().unwrap();

    fs::write(Path::new(repo).join("src/worker.rs"),
        "pub fn recover() -> bool { let result = true; result }\n").unwrap();
    let candidate = commit(Path::new(repo), "candidate");

    refused(&["investigation", "verify", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "3", "--plan-event", claim_event,
        "--candidate", &candidate, "--limit", "8"]);
    let verification = cli(&["investigation", "verify", "--repo", repo, "--db", db,
        "--id", id, "--expected-revision", "3", "--plan-event", plan_event,
        "--candidate", &candidate, "--limit", "8"]);
    assert_eq!(verification["kind"], "VERIFY");
    assert_eq!(verification["payload"]["report"]["schema"], "excession.agent-verification.v1");
    assert_eq!(verification["payload"]["report"]["base_commit"], base);
    assert_eq!(verification["payload"]["report"]["candidate_commit"], candidate);
    assert_eq!(verification["payload"]["report"]["ready_to_merge"], false);

    let history = cli(&["investigation", "show", "--repo", repo, "--db", db, "--id", id]);
    assert_eq!(history["schema"], "excession.investigation.v1");
    assert_eq!(history["revision"], 4);
    assert_eq!(history["events"].as_array().unwrap().len(), 4);
    assert_eq!(history["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(history["ready_to_merge"], false);

    let event_schema: serde_json::Value = serde_json::from_str(
        include_str!("../../contracts/product/investigation-event.v1.schema.json")
    ).unwrap();
    let report_schema: serde_json::Value = serde_json::from_str(
        include_str!("../../contracts/product/investigation.v1.schema.json")
    ).unwrap();
    let event_validator = jsonschema::draft202012::new(&event_schema).unwrap();
    let report_validator = jsonschema::draft202012::new(&report_schema).unwrap();
    for event in history["events"].as_array().unwrap() {
        assert!(event_validator.is_valid(event), "invalid event schema: {event}");
    }
    assert!(report_validator.is_valid(&history), "invalid report schema: {history}");
    let mut false_proof = history.clone();
    false_proof["semantic_verification"] = json!("PROVEN");
    assert!(!report_validator.is_valid(&false_proof));

    let database = Db::open(Path::new(db)).unwrap();
    assert!(database.execute(
        "UPDATE investigation_event_v1 SET revision=999 WHERE investigation_id=?",
        &[excession::sqlite::Param::Text(id)],
    ).is_err());
    assert!(database.execute(
        "DELETE FROM investigation_event_v1 WHERE investigation_id=?",
        &[excession::sqlite::Param::Text(id)],
    ).is_err());
}

#[test]
fn persisted_event_tampering_is_detected_and_unrelated_revisions_are_refused() {
    let (_tmp, repo, db_path, base) = repo();
    let db = Db::open(&db_path).unwrap();
    let first = investigation::start(&db, &repo, &base, "Which condition governs recovery?").unwrap();
    let id = first.get("investigation_id").unwrap().str().unwrap();
    let original = investigation::audit(&db, &repo, id).unwrap();
    assert_eq!(original.get("revision").unwrap().integer().unwrap(), 1);

    let wrong = "f".repeat(64);
    assert!(investigation::plan(&db, &repo, id, 1, &wrong, "preserve return value").is_err());
    assert!(investigation::start(&db, &repo, &base, "Which condition governs recovery?").is_err());

    db.exec("DROP TRIGGER investigation_no_update_v1;").unwrap();
    db.execute(
        "UPDATE investigation_event_v1 SET document=? WHERE investigation_id=? AND revision=1",
        &[excession::sqlite::Param::Text(&json!({"changed": true}).to_string()),
          excession::sqlite::Param::Text(id)],
    ).unwrap();
    assert!(investigation::audit(&db, &repo, id).is_err());
}
