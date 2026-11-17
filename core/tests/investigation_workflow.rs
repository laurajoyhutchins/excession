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
        .unwrap()
        .trim()
        .to_owned()
}

fn commit(root: &Path, title: &str) -> String {
    git(root, &["add", "-A"]);
    checked(Command::new("git").arg("-C").arg(root).args([
        "-c",
        "user.name=Excession Test",
        "-c",
        "user.email=excession@example.invalid",
        "commit",
        "-qm",
        title,
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
    fs::write(
        repo.join("src/recovery.rs"),
        "pub fn recover_worker() -> bool { retry_job(); true }\nfn retry_job() {}\n",
    )
    .unwrap();
    fs::write(repo.join("src/other.rs"), "pub fn display_canvas() {}\n").unwrap();
    fs::write(
        repo.join("tests/recovery_test.rs"),
        "#[test] fn recovery_test() { assert!(true); }\n",
    )
    .unwrap();
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
        .args(args)
        .output()
        .expect("run refusal");
    assert_eq!(
        output.status.code(),
        Some(2),
        "unexpected result: {output:?}"
    );
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
        "investigation",
        "start",
        "--repo",
        repo,
        "--db",
        db,
        "--rev",
        &base,
        "--question",
        "Where does recover_worker call retry_job?",
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let found = command(&[
        "investigation",
        "explore",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--limit",
        "8",
        "--index-segments",
        "256",
    ]);
    assert_eq!(found["kind"], "EXPLORE");
    assert_eq!(
        found["payload"]["report"]["schema"],
        "excession.agent-locate.v1"
    );
    assert_eq!(found["payload"]["report"]["source_commit"], base);
    assert!(
        found["payload"]["report"].get("cache").is_none(),
        "volatile local cache path must not be in persistent report"
    );
    assert_eq!(
        found["payload"]["report"]["safe_to_skip_unlisted_tests"],
        false
    );
    let reading = &found["payload"]["report"]["reading_plan"]["steps"];
    assert!(reading.is_array());
    assert!(
        reading
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["source"]["path"] == "src/recovery.rs"),
        "the actual recovery source should be in the inspection plan: {reading}"
    );

    refused(&[
        "investigation",
        "explore",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--limit",
        "8",
    ]);
    let question = command(&[
        "investigation",
        "question",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--question",
        "Does recovery retry after a persistence error?",
        "--reason",
        "Current source search cannot establish runtime error conditions",
    ]);
    assert_eq!(question["kind"], "QUESTION");

    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "3",
        "--path",
        "src/recovery.rs",
        "--statement",
        "recover_worker invokes retry_job",
        "--assumption",
        "Reported from source inspection without execution",
        "--assessment",
        "SOURCE_REVIEW_REPORTED",
    ]);
    let claim_id = claim["event_id"].as_str().unwrap();
    refused(&[
        "investigation",
        "synthesize",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--narrative",
        "The worker retries",
        "--claim-events",
        started["event_id"].as_str().unwrap(),
    ]);
    let synthesis = command(&[
        "investigation",
        "synthesize",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--narrative",
        "Source inspection suggests the recovery entry point delegates to retry_job",
        "--claim-events",
        claim_id,
    ]);
    assert_eq!(synthesis["kind"], "SYNTHESIS");
    assert_eq!(synthesis["payload"]["claim_event_ids"][0], claim_id);

    let explanation = command(&[
        "investigation",
        "explain",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
    ]);
    assert_eq!(
        explanation["schema"],
        "excession.investigation-explanation.v1"
    );
    assert_eq!(explanation["revision"], 5);
    assert_eq!(
        explanation["implementation_map"][0]["path"],
        "src/recovery.rs"
    );
    assert_eq!(explanation["claims"][0]["event_id"], claim_id);
    assert_eq!(explanation["narratives"][0]["claim_event_ids"][0], claim_id);
    assert_eq!(explanation["open_questions"][0]["state"], "OPEN");
    assert_eq!(
        explanation["retrievals"][0]["report_id"],
        found["payload"]["report"]["report_id"]
    );
    assert_eq!(
        explanation["completeness"],
        "INCOMPLETE_BEHAVIORAL_COVERAGE"
    );
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
    let history = command(&[
        "investigation",
        "show",
        "--repo",
        repo,
        "--db",
        db,
        "--id",
        id,
    ]);
    for event in history["events"].as_array().unwrap() {
        assert!(valid_event.is_valid(event), "event: {event}");
    }
    assert!(
        valid_view.is_valid(&explanation),
        "explanation: {explanation}"
    );
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
        "investigation",
        "start",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--rev",
        &base,
        "--question",
        "What must be preserved in recovery?",
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--path",
        "src/recovery.rs",
        "--statement",
        "recovery invokes retry_job",
        "--assumption",
        "source-level observation only",
        "--assessment",
        "SOURCE_REVIEW_REPORTED",
    ]);
    let claim_id = claim["event_id"].as_str().unwrap();

    fs::write(
        repo.join("src/other.rs"),
        "pub fn display_canvas() { println!(\"ok\"); }\n",
    )
    .unwrap();
    let unrelated = commit(&repo, "unrelated change");
    let first = command(&[
        "investigation",
        "refresh",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--candidate",
        &unrelated,
    ]);
    assert_eq!(
        first["payload"]["claim_assessments"][0]["claim_event_id"],
        claim_id
    );
    assert_eq!(
        first["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_UNCHANGED"
    );
    assert_eq!(
        first["payload"]["claim_assessments"][0]["semantic_impact"],
        "UNRESOLVED"
    );
    assert_eq!(
        first["payload"]["dependency_completeness"],
        "NOT_ESTABLISHED"
    );
    assert_eq!(first["payload"]["changed_path_count"], 1);

    fs::write(
        repo.join("src/recovery.rs"),
        "pub fn recover_worker() -> bool { false }\n",
    )
    .unwrap();
    let modified = commit(&repo, "changed recovery");
    let second = command(&[
        "investigation",
        "refresh",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
        "--expected-revision",
        "3",
        "--candidate",
        &modified,
    ]);
    assert_eq!(
        second["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_CHANGED"
    );

    fs::remove_file(repo.join("src/recovery.rs")).unwrap();
    let deleted = commit(&repo, "deleted recovery");
    let third = command(&[
        "investigation",
        "refresh",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--candidate",
        &deleted,
    ]);
    assert_eq!(
        third["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_UNAVAILABLE"
    );
    refused(&[
        "investigation",
        "refresh",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--candidate",
        &deleted,
    ]);

    let explanation = command(&[
        "investigation",
        "explain",
        "--repo",
        repo_str,
        "--db",
        db_str,
        "--id",
        id,
    ]);
    assert_eq!(explanation["latest_refresh"]["candidate_commit"], deleted);
    assert_eq!(
        explanation["latest_refresh"]["semantic_verification"],
        "NOT_ESTABLISHED"
    );

    let db = Db::open(&db_path).unwrap();
    assert!(
        db.execute(
            "DELETE FROM investigation_event_v1 WHERE investigation_id=?",
            &[excession::sqlite::Param::Text(id)]
        )
        .is_err()
    );
}

#[test]
fn incomplete_index_budgets_remain_visible_in_persisted_explanations() {
    let (_tmp, repo, db, _base) = setup();
    // A processing budget smaller than this authenticated source must
    // checkpoint an incomplete file, not silently omit it or report COMPLETE.
    fs::write(
        repo.join("src/recovery.rs"),
        format!(
            "{}pub fn recover_worker() -> bool {{ retry_job(); true }}\nfn retry_job() {{}}\n",
            "// source filler\n".repeat(100)
        ),
    )
    .unwrap();
    let base = commit(&repo, "large recovery source");
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let started = command(&[
        "investigation",
        "start",
        "--repo",
        r,
        "--db",
        d,
        "--rev",
        &base,
        "--question",
        "Where is recover_worker?",
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let explored = command(&[
        "investigation",
        "explore",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--limit",
        "8",
        "--index-segments",
        "256",
        "--max-file-bytes",
        "256",
        "--max-segment-bytes",
        "256",
        "--max-segment-files",
        "1",
    ]);
    assert_eq!(explored["kind"], "EXPLORE");
    assert_eq!(explored["payload"]["report"]["index"]["complete"], false);
    assert_eq!(
        explored["payload"]["report"]["index"]["limits"]["max_file_bytes"],
        256
    );
    let gaps = explored["payload"]["report"]["coverage"]["gaps"]
        .as_array()
        .unwrap();
    assert!(
        gaps.iter()
            .any(|gap| { gap["reason"] == "SOURCE_CONTENT_NOT_FULLY_INDEXED" }),
        "incomplete source index must be explicit: {gaps:?}"
    );
    let explanation = command(&[
        "investigation",
        "explain",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
    ]);
    assert_eq!(explanation["retrievals"][0]["index_complete"], false);
    assert!(
        explanation["evidence_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| {
                gap["reason"] == "SOURCE_CONTENT_NOT_FULLY_INDEXED" && gap["state"] == "UNRESOLVED"
            })
    );
    assert_eq!(
        explanation["completeness"],
        "INCOMPLETE_BEHAVIORAL_COVERAGE"
    );

    refused(&[
        "investigation",
        "explore",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--max-file-bytes",
        "0",
    ]);
    assert_eq!(
        command(&["investigation", "show", "--repo", r, "--db", d, "--id", id])["revision"],
        2
    );
}

#[test]
fn source_behavior_witnesses_bind_control_data_state_to_git_and_preservation_plans() {
    let (_tmp, repo, db, _) = setup();
    fs::write(
        repo.join("src/flow.rs"),
        "fn process(ready: bool, input: u32, counter: &mut u32) -> u32 {\n\
         \x20\x20  if !ready { return 0; }\n\
         \x20\x20  let result = input + 1;\n\
         \x20\x20  *counter += 1;\n\
         \x20\x20  result\n}\n",
    )
    .unwrap();
    let base = commit(&repo, "introduce flow fixture");
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let first = command(&[
        "investigation",
        "start",
        "--repo",
        r,
        "--db",
        d,
        "--rev",
        &base,
        "--question",
        "Which control, data and state operations must survive?",
    ]);
    let id = first["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--path",
        "src/flow.rs",
        "--statement",
        "The source contains a conditional, a value assignment, and a counter mutation",
        "--assumption",
        "Source syntax is not executable behavior verification",
        "--assessment",
        "SOURCE_REVIEW_REPORTED",
    ]);
    let claim_id = claim["event_id"].as_str().unwrap();

    let witnesses = [
        (
            "2",
            "CONTROL_CONDITION",
            "The source has an early return guard",
        ),
        (
            "3",
            "DATA_TRANSFER",
            "The source computes result from input",
        ),
        ("4", "STATE_MUTATION", "The source updates the counter"),
    ];
    let mut ids = Vec::new();
    for (i, (line, kind, meaning)) in witnesses.iter().enumerate() {
        let event = command(&[
            "investigation",
            "witness",
            "--repo",
            r,
            "--db",
            d,
            "--id",
            id,
            "--expected-revision",
            &(i + 2).to_string(),
            "--claim-event",
            claim_id,
            "--first-line",
            line,
            "--last-line",
            line,
            "--kind",
            kind,
            "--interpretation",
            meaning,
        ]);
        assert_eq!(event["kind"], "WITNESS");
        assert_eq!(event["payload"]["kind"], *kind);
        assert_eq!(event["payload"]["source"]["commit"], base);
        assert_eq!(event["payload"]["runtime_verified"], false);
        assert_eq!(
            event["payload"]["assessment"],
            "SOURCE_TEXT_VERIFIED_INTERPRETATION_UNVERIFIED"
        );
        assert!(
            event["payload"]["source_text"]
                .as_str()
                .unwrap()
                .contains(match *kind {
                    "CONTROL_CONDITION" => "if !ready",
                    "DATA_TRANSFER" => "input + 1",
                    _ => "*counter += 1",
                })
        );
        ids.push(event["event_id"].as_str().unwrap().to_owned());
    }

    refused(&[
        "investigation",
        "witness",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--claim-event",
        claim_id,
        "--first-line",
        "4",
        "--last-line",
        "4",
        "--kind",
        "OBSERVED_RUNTIME_STATE",
        "--interpretation",
        "Not a valid source classification",
    ]);
    refused(&[
        "investigation",
        "witness",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--claim-event",
        claim_id,
        "--first-line",
        "99",
        "--last-line",
        "99",
        "--kind",
        "STATE_MUTATION",
        "--interpretation",
        "A nonexistent source line",
    ]);
    refused(&[
        "investigation",
        "witness",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--claim-event",
        first["event_id"].as_str().unwrap(),
        "--first-line",
        "2",
        "--last-line",
        "2",
        "--kind",
        "CONTROL_CONDITION",
        "--interpretation",
        "Wrong kind of reference",
    ]);

    let plan = command(&[
        "investigation",
        "plan",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--claim-event",
        claim_id,
        "--obligation",
        "Preserve the early-return guard, computed value, and counter mutation",
    ]);
    assert_eq!(plan["kind"], "PLAN");
    let explanation = command(&[
        "investigation",
        "explain",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
    ]);
    assert_eq!(explanation["revision"], 6);
    assert_eq!(
        explanation["behavior_witnesses"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        explanation["preservation_plans"][0]["witness_event_ids"],
        json!(ids)
    );
    assert_eq!(explanation["preservation_plans"][0]["state"], "OPEN");
    assert_eq!(explanation["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(explanation["ready_to_merge"], false);

    let event_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-event.v1.schema.json"
    ))
    .unwrap();
    let explanation_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-explanation.v1.schema.json"
    ))
    .unwrap();
    let event_validator = jsonschema::draft202012::new(&event_schema).unwrap();
    let explanation_validator = jsonschema::draft202012::new(&explanation_schema).unwrap();
    let history = command(&["investigation", "show", "--repo", r, "--db", d, "--id", id]);
    for event in history["events"].as_array().unwrap() {
        assert!(
            event_validator.is_valid(event),
            "bad persisted event: {event}"
        );
    }
    assert!(
        explanation_validator.is_valid(&explanation),
        "bad explanation: {explanation}"
    );

    fs::write(repo.join("src/flow.rs"), "fn process() -> u32 { 0 }\n").unwrap();
    let candidate = commit(&repo, "change flow implementation");
    let refresh = command(&[
        "investigation",
        "refresh",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "6",
        "--candidate",
        &candidate,
    ]);
    assert_eq!(
        refresh["payload"]["claim_assessments"][0]["direct_source"],
        "DIRECT_SOURCE_CHANGED"
    );
    assert_eq!(
        refresh["payload"]["claim_assessments"][0]["semantic_impact"],
        "UNRESOLVED"
    );
    assert_eq!(
        command(&[
            "investigation",
            "explain",
            "--repo",
            r,
            "--db",
            d,
            "--id",
            id
        ])["behavior_witnesses"][0]["runtime_verified"],
        false
    );

    let ledger = Db::open(&db).unwrap();
    ledger
        .exec("DROP TRIGGER investigation_no_update_v1;")
        .unwrap();
    ledger.execute(
        "UPDATE investigation_event_v1 SET document = REPLACE(document, 'input + 1', 'input + 9') \
         WHERE investigation_id=? AND revision=4",
        &[excession::sqlite::Param::Text(id)],
    ).unwrap();
    refused(&["investigation", "show", "--repo", r, "--db", d, "--id", id]);
}

#[test]
fn source_witness_streams_complete_large_blobs_with_bounded_span_memory() {
    let (_tmp, repo, db, _) = setup();
    let big = format!("{}{}\n", "# ignored\n".repeat(160_000), "state = state + 1");
    fs::write(repo.join("src/large.py"), big).unwrap();
    let base = commit(&repo, "introduce large Git source");
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let first = command(&[
        "investigation",
        "start",
        "--repo",
        r,
        "--db",
        d,
        "--rev",
        &base,
        "--question",
        "What source modifies the state?",
    ]);
    let id = first["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--path",
        "src/large.py",
        "--statement",
        "State is incremented",
        "--assumption",
        "Only source text is authenticated",
        "--assessment",
        "HYPOTHESIS",
    ]);
    let witness = command(&[
        "investigation",
        "witness",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--claim-event",
        claim["event_id"].as_str().unwrap(),
        "--first-line",
        "160001",
        "--last-line",
        "160001",
        "--kind",
        "STATE_MUTATION",
        "--interpretation",
        "Possible state update",
    ]);
    assert_eq!(witness["payload"]["source_text"], "state = state + 1\n");
    assert_eq!(witness["payload"]["runtime_verified"], false);
    let replay = command(&["investigation", "show", "--repo", r, "--db", d, "--id", id]);
    assert_eq!(replay["revision"], 3);
}

fn local_python() -> String {
    let output = checked(Command::new("which").arg("python3"));
    let path = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    assert!(
        Path::new(&path).is_absolute(),
        "Python must resolve to an absolute executable"
    );
    path
}

#[test]
fn differential_execution_binds_real_outputs_to_witness_backed_preservation_plan() {
    let (_tmp, repo, db, _initial) = setup();
    fs::write(
        repo.join("src/counter.py"),
        "def update(x):\n    state = 0\n    if x > 2:\n        state += x\n    return state\nprint(update(3))\n",
    ).unwrap();
    let base = commit(&repo, "baseline behavior");
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let py = local_python();
    let args = r#"["-B","src/counter.py"]"#;
    let started = command(&[
        "investigation",
        "start",
        "--repo",
        r,
        "--db",
        d,
        "--rev",
        &base,
        "--question",
        "What happens to the counter for input three?",
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--path",
        "src/counter.py",
        "--statement",
        "The source conditionally updates a counter",
        "--assumption",
        "Source reviewed, no runtime proof",
        "--assessment",
        "SOURCE_REVIEW_REPORTED",
    ]);
    let witness = command(&[
        "investigation",
        "witness",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--claim-event",
        claim["event_id"].as_str().unwrap(),
        "--first-line",
        "4",
        "--last-line",
        "4",
        "--kind",
        "STATE_MUTATION",
        "--interpretation",
        "Counter may increase",
    ]);
    let plan = command(&[
        "investigation",
        "plan",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "3",
        "--claim-event",
        claim["event_id"].as_str().unwrap(),
        "--obligation",
        "Preserve the output for the given input three",
    ]);
    let plan_id = plan["event_id"].as_str().unwrap();

    // A source revision with only an implementation comment gives the same result.
    fs::write(
        repo.join("src/counter.py"),
        "# Implementation note\n\
         def update(x):\n    state = 0\n    if x > 2:\n        state += x\n    return state\nprint(update(3))\n",
    ).unwrap();
    let same_output = commit(&repo, "comment-only source change");
    refused(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--plan-event",
        plan_id,
        "--candidate",
        &same_output,
        "--program",
        &py,
        "--argv-json",
        args,
        "--trusted-local",
        "false",
    ]);
    let agreed = command(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "4",
        "--plan-event",
        plan_id,
        "--candidate",
        &same_output,
        "--program",
        &py,
        "--argv-json",
        args,
        "--trusted-local",
        "true",
        "--timeout-ms",
        "12000",
        "--max-output-bytes",
        "8192",
    ]);
    assert_eq!(agreed["kind"], "EXECUTION");
    let report = &agreed["payload"]["report"];
    assert_eq!(report["schema"], "excession.investigation-execution.v1");
    assert_eq!(report["base_commit"], base);
    assert_eq!(report["candidate_commit"], same_output);
    assert_eq!(report["comparison"], "AGREEMENT_OBSERVED");
    assert_eq!(report["trust"], "TRUSTED_LOCAL_EXECUTION_UNATTESTED");
    assert_eq!(report["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(report["ready_to_merge"], false);
    assert_eq!(report["witness_event_ids"], json!([witness["event_id"]]));
    assert_eq!(report["base"]["status"], "EXITED");
    assert_eq!(report["candidate"]["exit_code"], 0);
    assert_eq!(
        report["base"]["stdout_sha256"],
        report["candidate"]["stdout_sha256"]
    );

    // A different stdout on the same command and input is directly observable.
    fs::write(
        repo.join("src/counter.py"),
        "def update(x):\n    state = 0\n    if x > 2:\n        state += x + 1\n    return state\nprint(update(3))\n",
    ).unwrap();
    let changed = commit(&repo, "change counter output");
    let differed = command(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--plan-event",
        plan_id,
        "--candidate",
        &changed,
        "--program",
        &py,
        "--argv-json",
        args,
        "--trusted-local",
        "true",
        "--timeout-ms",
        "12000",
        "--max-output-bytes",
        "8192",
    ]);
    assert_eq!(
        differed["payload"]["report"]["comparison"],
        "DIFFERENCE_OBSERVED"
    );
    assert_ne!(
        differed["payload"]["report"]["base"]["stdout_sha256"],
        differed["payload"]["report"]["candidate"]["stdout_sha256"]
    );
    refused(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "5",
        "--plan-event",
        plan_id,
        "--candidate",
        &changed,
        "--program",
        &py,
        "--argv-json",
        args,
        "--trusted-local",
        "true",
    ]);

    // Explicit finite output budgets yield INCONCLUSIVE, not synthetic agreement.
    let limited = command(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "6",
        "--plan-event",
        plan_id,
        "--candidate",
        &changed,
        "--program",
        &py,
        "--argv-json",
        args,
        "--trusted-local",
        "true",
        "--max-output-bytes",
        "1",
        "--timeout-ms",
        "12000",
    ]);
    assert_eq!(limited["payload"]["report"]["comparison"], "INCONCLUSIVE");
    assert_eq!(
        limited["payload"]["report"]["base"]["status"],
        "OUTPUT_LIMIT"
    );
    assert_eq!(
        limited["payload"]["report"]["candidate"]["status"],
        "OUTPUT_LIMIT"
    );
    refused(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "7",
        "--plan-event",
        plan_id,
        "--candidate",
        &changed,
        "--program",
        &py,
        "--argv-json",
        "not-json",
        "--trusted-local",
        "true",
    ]);

    let history = command(&["investigation", "show", "--repo", r, "--db", d, "--id", id]);
    assert_eq!(history["revision"], 7);
    let projection = command(&[
        "investigation",
        "explain",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
    ]);
    assert_eq!(
        projection["execution_comparisons"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(projection["preservation_plans"][0]["state"], "OPEN");
    assert_eq!(
        projection["behavior_witnesses"][0]["runtime_verified"],
        false
    );
    assert_eq!(projection["semantic_verification"], "NOT_ESTABLISHED");

    let event_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-event.v1.schema.json"
    ))
    .unwrap();
    let exec_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-execution.v1.schema.json"
    ))
    .unwrap();
    let view_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-explanation.v1.schema.json"
    ))
    .unwrap();
    let event_validator = jsonschema::draft202012::new(&event_schema).unwrap();
    let exec_validator = jsonschema::draft202012::new(&exec_schema).unwrap();
    let view_validator = jsonschema::draft202012::new(&view_schema).unwrap();
    for row in history["events"].as_array().unwrap() {
        assert!(event_validator.is_valid(row), "event: {row}");
    }
    assert!(
        exec_validator.is_valid(report),
        "execution report: {report}"
    );
    assert!(
        view_validator.is_valid(&projection),
        "projection: {projection}"
    );
    let mut forged = report.clone();
    forged["semantic_verification"] = json!("PROVEN");
    assert!(!exec_validator.is_valid(&forged));
}

#[test]
fn execution_requires_prior_witness_and_plan_and_never_runs_by_default() {
    let (_tmp, repo, db, base) = setup();
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let started = command(&[
        "investigation",
        "start",
        "--repo",
        r,
        "--db",
        d,
        "--rev",
        &base,
        "--question",
        "What does the worker return?",
    ]);
    let id = started["investigation_id"].as_str().unwrap();
    let claim = command(&[
        "investigation",
        "claim",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "1",
        "--path",
        "src/recovery.rs",
        "--statement",
        "The source invokes recovery",
        "--assumption",
        "Only a source claim",
        "--assessment",
        "HYPOTHESIS",
    ]);
    let plan = command(&[
        "investigation",
        "plan",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "2",
        "--claim-event",
        claim["event_id"].as_str().unwrap(),
        "--obligation",
        "Preserve behavior",
    ]);
    fs::write(repo.join("src/other.rs"), "pub fn changed() {}\n").unwrap();
    let candidate = commit(&repo, "unrelated candidate change");
    let py = local_python();
    refused(&[
        "investigation",
        "execute",
        "--repo",
        r,
        "--db",
        d,
        "--id",
        id,
        "--expected-revision",
        "3",
        "--plan-event",
        plan["event_id"].as_str().unwrap(),
        "--candidate",
        &candidate,
        "--program",
        &py,
        "--argv-json",
        r#"["-c","print(2)"]"#,
        "--trusted-local",
        "true",
    ]);
    assert_eq!(
        command(&["investigation", "show", "--repo", r, "--db", d, "--id", id])["revision"],
        3
    );
}


#[test]
fn runtime_trace_observes_lines_local_changes_and_internal_behavior_differences() {
    let (_tmp, repo, db, _) = setup();
    fs::write(repo.join("src/counter.py"),
        "def counter(x):\n    state = x\n    if state > 2:\n        state += 1\n    return state\nprint(counter(3))\n"
    ).unwrap();
    let base = commit(&repo, "counter baseline");
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let interpreter = local_python();
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tools/python_trace_driver.py")
        .canonicalize().unwrap();
    let driver = driver.to_str().unwrap();
    let first = command(&["investigation","start","--repo",r,"--db",d,
        "--rev",&base,"--question","Which source-level state changes are visited?"]);
    let id = first["investigation_id"].as_str().unwrap();
    let claim = command(&["investigation","claim","--repo",r,"--db",d,
        "--id",id,"--expected-revision","1","--path","src/counter.py",
        "--statement","counter conditionally updates state",
        "--assumption","A source claim is not a runtime proof",
        "--assessment","SOURCE_REVIEW_REPORTED"]);
    let witness = command(&["investigation","witness","--repo",r,"--db",d,
        "--id",id,"--expected-revision","2",
        "--claim-event",claim["event_id"].as_str().unwrap(),
        "--first-line","4","--last-line","4","--kind","STATE_MUTATION",
        "--interpretation","This line may increase a scalar local"]);
    let plan = command(&["investigation","plan","--repo",r,"--db",d,
        "--id",id,"--expected-revision","3",
        "--claim-event",claim["event_id"].as_str().unwrap(),
        "--obligation","Preserve observable counter output and check state-update visits"]);
    let plan_id = plan["event_id"].as_str().unwrap();

    fs::write(repo.join("src/other.rs"), "pub fn changed_without_counter() {}\n").unwrap();
    let same = commit(&repo, "unrelated source edit");
    refused(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","4","--plan-event",plan_id,
        "--candidate",&same,"--program",&interpreter,"--driver",driver,
        "--script","src/counter.py","--argv-json","[]","--trusted-local","false"]);
    let traced = command(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","4","--plan-event",plan_id,
        "--candidate",&same,"--program",&interpreter,"--driver",driver,
        "--script","src/counter.py","--argv-json","[]","--trusted-local","true",
        "--timeout-ms","12000","--max-trace-events","2000",
        "--max-trace-bytes","1048576","--max-output-bytes","8192"]);
    assert_eq!(traced["kind"], "TRACE");
    let report = &traced["payload"]["report"];
    assert_eq!(report["schema"], "excession.investigation-runtime-trace.v1");
    assert_eq!(report["process_comparison"], "AGREEMENT_OBSERVED");
    assert_eq!(report["trace_comparison"], "MATCHING_REPORTED_EVENTS");
    assert_eq!(report["base_trace"]["status"], "REPORTED_COMPLETE");
    assert_eq!(report["candidate_trace"]["status"], "REPORTED_COMPLETE");
    assert_eq!(report["base_witness_line_hits"][0]["witness_event_id"], witness["event_id"]);
    assert!(report["base_witness_line_hits"][0]["base_line_event_count"].as_u64().unwrap() > 0);
    assert_eq!(report["base_witness_line_hits"][0]["interpretation_verified"], false);
    let events = report["base_trace"]["events"].as_array().unwrap();
    assert!(events.iter().any(|event| event["kind"] == "CALL"
        && event["function"] == "counter"), "runtime CALL missing: {events:?}");
    assert!(events.iter().any(|event| event["kind"] == "LINE"
        && event["path"] == "src/counter.py" && event["line"] == 4),
        "conditional body line was not observed");
    assert!(events.iter().any(|event| event["changes"].as_array().unwrap().iter().any(
        |change| change["name"] == "state"
            && change["before"] != change["after"]
    )), "local scalar change was not captured");
    assert_eq!(report["trust"], "BEST_EFFORT_TRUSTED_LOCAL_CPYTHON_UNATTESTED");
    assert_eq!(report["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(report["ready_to_merge"], false);

    // The stdout is unchanged, but an additional internal state transition
    // gives a different instrumented event sequence.
    fs::write(repo.join("src/counter.py"),
        "def counter(x):\n    state = x\n    if state > 2:\n        state += 1\n        state -= 1\n        state += 1\n    return state\nprint(counter(3))\n"
    ).unwrap();
    let different_internal = commit(&repo, "add intermediate state transitions");
    let second = command(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","5","--plan-event",plan_id,
        "--candidate",&different_internal,"--program",&interpreter,
        "--driver",driver,"--script","src/counter.py","--argv-json","[]",
        "--trusted-local","true","--timeout-ms","12000",
        "--max-trace-events","2000","--max-trace-bytes","1048576"]);
    assert_eq!(second["payload"]["report"]["process_comparison"], "AGREEMENT_OBSERVED");
    assert_eq!(second["payload"]["report"]["trace_comparison"], "DIFFERING_REPORTED_EVENTS");
    assert_eq!(second["payload"]["report"]["base_trace"]["status"], "REPORTED_COMPLETE");

    let incomplete = command(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","6","--plan-event",plan_id,
        "--candidate",&different_internal,"--program",&interpreter,
        "--driver",driver,"--script","src/counter.py","--argv-json","[]",
        "--trusted-local","true","--timeout-ms","12000",
        "--max-trace-events","2","--max-trace-bytes","1048576"]);
    assert_eq!(incomplete["payload"]["report"]["base_trace"]["status"], "TRUNCATED");
    assert_eq!(incomplete["payload"]["report"]["trace_comparison"], "INCONCLUSIVE");

    let view = command(&["investigation","explain","--repo",r,"--db",d,"--id",id]);
    assert_eq!(view["runtime_traces"].as_array().unwrap().len(), 3);
    assert_eq!(view["preservation_plans"][0]["state"], "OPEN");
    assert_eq!(view["behavior_witnesses"][0]["runtime_verified"], false);
    assert_eq!(view["semantic_verification"], "NOT_ESTABLISHED");
    assert_eq!(view["ready_to_merge"], false);
    let event_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-event.v1.schema.json"
    )).unwrap();
    let trace_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-runtime-trace.v1.schema.json"
    )).unwrap();
    let view_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/product/investigation-explanation.v1.schema.json"
    )).unwrap();
    let event_validator = jsonschema::draft202012::new(&event_schema).unwrap();
    let trace_validator = jsonschema::draft202012::new(&trace_schema).unwrap();
    let view_validator = jsonschema::draft202012::new(&view_schema).unwrap();
    let history = command(&["investigation","show","--repo",r,"--db",d,"--id",id]);
    for event in history["events"].as_array().unwrap() {
        assert!(event_validator.is_valid(event), "event: {event}");
    }
    assert!(trace_validator.is_valid(report), "trace report: {report}");
    assert!(view_validator.is_valid(&view), "view: {view}");
    let mut forged = report.clone();
    forged["semantic_verification"] = json!("PROVEN");
    assert!(!trace_validator.is_valid(&forged));
    refused(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","6","--plan-event",plan_id,
        "--candidate",&different_internal,"--program",&interpreter,
        "--driver",driver,"--script","src/counter.py","--argv-json","[]",
        "--trusted-local","true"]);
}

#[test]
fn runtime_trace_refuses_unsupported_source_and_unwitnessed_plan() {
    let (_tmp, repo, db, base) = setup();
    let r = repo.to_str().unwrap();
    let d = db.to_str().unwrap();
    let py = local_python();
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tools/python_trace_driver.py").canonicalize().unwrap();
    let driver = driver.to_str().unwrap();
    let first = command(&["investigation","start","--repo",r,"--db",d,
        "--rev",&base,"--question","Is worker behavior preserved?"]);
    let id = first["investigation_id"].as_str().unwrap();
    let claim = command(&["investigation","claim","--repo",r,"--db",d,
        "--id",id,"--expected-revision","1","--path","src/recovery.rs",
        "--statement","The worker source has a return",
        "--assumption","Source-only claim","--assessment","HYPOTHESIS"]);
    let plan = command(&["investigation","plan","--repo",r,"--db",d,
        "--id",id,"--expected-revision","2",
        "--claim-event",claim["event_id"].as_str().unwrap(),
        "--obligation","Preserve return behavior"]);
    fs::write(repo.join("src/other.rs"), "pub fn modified() {}\n").unwrap();
    let candidate = commit(&repo, "candidate change");
    refused(&["investigation","trace","--repo",r,"--db",d,
        "--id",id,"--expected-revision","3","--plan-event",
        plan["event_id"].as_str().unwrap(),"--candidate",&candidate,
        "--program",&py,"--driver",driver,
        "--script","src/unknown.py","--argv-json","[]","--trusted-local","true"]);
    assert_eq!(command(&["investigation","show","--repo",r,"--db",d,"--id",id])["revision"], 3);
}
