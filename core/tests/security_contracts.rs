//! End-to-end security ingestion and independently replayed audit contracts.
//! These tests use real Git revisions and the native SQLite binding.
use excession::evidence::identity::SourceRef;
use excession::json::{self, Value};
use excession::sqlite::Db;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn success(out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn refused(out: Output) {
    assert!(!out.status.success(), "unexpected acceptance");
    assert!(
        out.stdout.is_empty(),
        "refusal must not emit successful output"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
}
fn parsed(out: Output) -> Value {
    json::parse(&success(out)).unwrap()
}
fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).unwrap().str().unwrap()
}
fn field_mut<'a>(v: &'a mut Value, key: &str) -> &'a mut Value {
    v.entries_mut().unwrap().get_mut(key).unwrap()
}
fn item_mut(v: &mut Value) -> &mut Value {
    match v {
        Value::Array(xs) => &mut xs[0],
        _ => panic!("array expected"),
    }
}
fn first_result(v: &mut Value) -> &mut Value {
    item_mut(field_mut(item_mut(field_mut(v, "runs")), "results"))
}
fn codeql(line: i64) -> Value {
    let report = format!(
        r#"{{"version":"2.1.0","$schema":"https://json.schemastore.org/sarif-2.1.0.json","runs":[{{"tool":{{"driver":{{"name":"CodeQL","version":"2.22.0"}}}},"results":[{{"ruleId":"go/path-injection","message":{{"text":"candidate only"}},"locations":[{{"physicalLocation":{{"artifactLocation":{{"uri":"app.go"}},"region":{{"startLine":{line}}}}}}}]}}]}}]}}"#
    );
    json::parse(report.as_bytes()).unwrap()
}
fn semgrep() -> Value {
    json::parse(br#"{"version":"1.0.0","results":[{"check_id":"test.rule","path":"./app.go","start":{"line":3},"extra":{"message":"bounded candidate","severity":"WARNING"}}],"errors":[]}"#).unwrap()
}
fn source_step(path: &str, line: i64) -> Value {
    json::parse(format!(r#"{{"location":{{"physicalLocation":{{"artifactLocation":{{"uri":"{path}"}},"region":{{"startLine":{line}}}}}}}}}"#).as_bytes()).unwrap()
}
struct Fixture {
    temp: PathBuf,
    repo: PathBuf,
    report: PathBuf,
    db: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}
impl Fixture {
    fn new() -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-security-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = temp.join("project");
        fs::create_dir_all(&repo).unwrap();
        let root = repo.to_str().unwrap();
        success(run("git", &["-C", root, "init", "-q", "-b", "main"]));
        success(run("git", &["-C", root, "config", "user.name", "Test"]));
        success(run(
            "git",
            &[
                "-C",
                root,
                "config",
                "user.email",
                "excession-test@example.invalid",
            ],
        ));
        fs::write(
            repo.join("app.go"),
            "package app\nfunc Process(x string) string {\n return x\n}\n",
        )
        .unwrap();
        success(run("git", &["-C", root, "add", "-A"]));
        success(run(
            "git",
            &["-C", root, "commit", "-qm", "source snapshot"],
        ));
        let report = temp.join("report.json");
        let db = temp.join("findings.sqlite");
        Self {
            temp,
            repo,
            report,
            db,
        }
    }
    fn commit(&self) {
        success(run(
            "git",
            &["-C", self.repo.to_str().unwrap(), "add", "-A"],
        ));
        success(run(
            "git",
            &[
                "-C",
                self.repo.to_str().unwrap(),
                "commit",
                "-qm",
                "source snapshot",
            ],
        ));
    }
    fn report(&self, data: &Value) {
        fs::write(&self.report, data.canonical()).unwrap();
    }
    fn ing(&self, engine: &str, audit: bool) -> Output {
        cli(&[
            "security",
            if audit { "audit" } else { "ingest" },
            "--repo",
            self.repo.to_str().unwrap(),
            "--report",
            self.report.to_str().unwrap(),
            "--db",
            self.db.to_str().unwrap(),
            "--engine",
            engine,
        ])
    }
    fn database(&self) -> Db {
        Db::open(&self.db).unwrap()
    }
}
#[test]
fn codeql_replay_only_confirms_unverified_candidates() {
    let f = Fixture::new();
    f.report(&codeql(2));
    let first = parsed(f.ing("codeql", false));
    let replay = parsed(f.ing("codeql", true));
    assert_eq!(first.get("run_id").unwrap(), replay.get("run_id").unwrap());
    assert_eq!(replay.get("candidate_count").unwrap().integer().unwrap(), 1);
    assert_eq!(str_at(&replay, "status"), "UNVERIFIED_STATIC_CANDIDATES");
    assert!(
        replay
            .get("independently_replayed")
            .unwrap()
            .bool()
            .unwrap()
    );
    let found = f
        .database()
        .query("SELECT candidate_id FROM security_ingest_candidate", &[])
        .unwrap();
    assert_eq!(found.len(), 1);
}
#[test]
fn scanner_source_identity_agrees_with_shared_git_subject() {
    let f = Fixture::new();
    f.report(&semgrep());
    parsed(f.ing("semgrep", false));
    let commit = success(run("git", &["-C", f.repo.to_str().unwrap(), "rev-parse", "HEAD"]));
    let commit = String::from_utf8(commit).unwrap();
    let source = SourceRef::from_git(&f.repo, commit.trim(), "app.go").unwrap();
    let stored = f.database().query(
        "SELECT source_blob FROM security_ingest_candidate WHERE source_path='app.go'",
        &[],
    ).unwrap();
    assert_eq!(stored[0][0].as_deref(), Some(source.blob.as_str()));
    parsed(f.ing("semgrep", true));
}
#[test]
fn codeql_flow_steps_are_source_bound_and_sql_mutations_refused() {
    let f = Fixture::new();
    let mut report = codeql(2);
    let steps = Value::array([source_step("app.go", 2), source_step("app.go", 3)]);
    let path_flow = json::parse(br#"{"threadFlows":[{"locations":[]}]}"#).unwrap();
    let mut path_flow = path_flow;
    let thread = item_mut(field_mut(&mut path_flow, "threadFlows"));
    field_mut(thread, "locations").clone_from(&steps);
    first_result(&mut report)
        .entries_mut()
        .unwrap()
        .insert("codeFlows".into(), Value::array([path_flow]));
    f.report(&report);
    let imported = parsed(f.ing("codeql", false));
    assert_eq!(
        imported.get("flow_step_count").unwrap().integer().unwrap(),
        2
    );
    parsed(f.ing("codeql", true));
    let db = f.database();
    let steps = db.query(
        "SELECT source_path,line,source_blob FROM security_ingest_flow_step ORDER BY step_index", &[]).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0][0].as_deref(), Some("app.go"));
    assert_eq!(steps[0][1].as_deref(), Some("2"));
    assert_eq!(steps[1][1].as_deref(), Some("3"));
    assert_eq!(steps[0][2], steps[1][2]);
    db.exec("UPDATE security_ingest_flow_step SET line=1 WHERE step_index=1")
        .unwrap();
    refused(f.ing("codeql", true));
}
#[test]
fn codeql_rejects_flow_to_untracked_source() {
    let f = Fixture::new();
    let mut report = codeql(2);
    let mut flow = json::parse(br#"{"threadFlows":[{"locations":[]}]}"#).unwrap();
    field_mut(item_mut(field_mut(&mut flow, "threadFlows")), "locations")
        .clone_from(&Value::array([source_step("hidden.go", 1)]));
    first_result(&mut report)
        .entries_mut()
        .unwrap()
        .insert("codeFlows".into(), Value::array([flow]));
    f.report(&report);
    refused(f.ing("codeql", false));
}
#[test]
fn semgrep_ingest_and_replay() {
    let f = Fixture::new();
    f.report(&semgrep());
    parsed(f.ing("semgrep", false));
    parsed(f.ing("semgrep", true));
}
#[test]
fn mutation_of_candidate_sql_is_detected() {
    let f = Fixture::new();
    f.report(&semgrep());
    parsed(f.ing("semgrep", false));
    f.database()
        .exec("UPDATE security_ingest_candidate SET rule_id='forged'")
        .unwrap();
    refused(f.ing("semgrep", true));
}
#[test]
fn deleted_evidence_sql_is_detected() {
    let f = Fixture::new();
    f.report(&codeql(2));
    parsed(f.ing("codeql", false));
    f.database()
        .exec("DELETE FROM security_ingest_candidate")
        .unwrap();
    refused(f.ing("codeql", true));
}
#[test]
fn changed_or_untracked_checkout_is_rejected() {
    let f = Fixture::new();
    f.report(&semgrep());
    parsed(f.ing("semgrep", false));
    fs::write(f.repo.join("app.go"), "package app\nfunc Evil() {}\n").unwrap();
    refused(f.ing("semgrep", true));
    let g = Fixture::new();
    g.report(&semgrep());
    fs::write(g.repo.join("something.go"), "package app\n").unwrap();
    refused(g.ing("semgrep", false));
}
#[test]
fn unsafe_uris_and_invalid_line_numbers_are_refused() {
    for uri in ["/etc/passwd", "../outside.go", "file:///app.go", "app%2Ego"] {
        let f = Fixture::new();
        let mut report = codeql(2);
        let first = item_mut(field_mut(first_result(&mut report), "locations"));
        let location = field_mut(first, "physicalLocation");
        let artifact = field_mut(location, "artifactLocation");
        field_mut(artifact, "uri").clone_from(&Value::string(uri));
        f.report(&report);
        refused(f.ing("codeql", false));
    }
    let f = Fixture::new();
    f.report(&codeql(999));
    refused(f.ing("codeql", false));
}
#[test]
fn scanner_failures_are_not_treated_as_clean_reports() {
    let f = Fixture::new();
    let mut report = semgrep();
    field_mut(&mut report, "errors").clone_from(&Value::array([json::parse(
        br#"{"message":"failed to parse target"}"#,
    )
    .unwrap()]));
    f.report(&report);
    refused(f.ing("semgrep", false));

    let g = Fixture::new();
    let mut report = codeql(2);
    item_mut(field_mut(&mut report, "runs"))
        .entries_mut()
        .unwrap()
        .insert(
            "invocations".into(),
            Value::array([json::parse(br#"{"executionSuccessful":false}"#).unwrap()]),
        );
    g.report(&report);
    refused(g.ing("codeql", false));
}
#[test]
fn wrong_scanner_name_and_duplicate_json_keys_are_refused() {
    let f = Fixture::new();
    let mut report = codeql(2);
    field_mut(
        field_mut(item_mut(field_mut(&mut report, "runs")), "tool"),
        "driver",
    )
    .entries_mut()
    .unwrap()
    .insert("name".into(), Value::string("Other"));
    f.report(&report);
    refused(f.ing("codeql", false));
    fs::write(
        &f.report,
        r#"{"version":"2.1.0","version":"2.1.0","runs":[]}"#,
    )
    .unwrap();
    refused(f.ing("codeql", false));
}
#[test]
fn separate_revisions_preserve_unverified_candidate_history() {
    let f = Fixture::new();
    f.report(&semgrep());
    let before = parsed(f.ing("semgrep", false));
    fs::write(
        f.repo.join("app.go"),
        "package app\nfunc Process(x string) string {\n return x + \"a\"\n}\n",
    )
    .unwrap();
    f.commit();
    let empty = json::parse(br#"{"version":"1.0","results":[],"errors":[]}"#).unwrap();
    let report = f.temp.join("second.json");
    fs::write(&report, empty.canonical()).unwrap();
    let args = |mode| {
        vec![
            "security",
            mode,
            "--repo",
            f.repo.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--db",
            f.db.to_str().unwrap(),
            "--engine",
            "semgrep",
        ]
    };
    let after = parsed(cli(&args("ingest")));
    assert_eq!(after.get("candidate_count").unwrap().integer().unwrap(), 0);
    parsed(cli(&args("audit")));
    assert_ne!(str_at(&before, "run_id"), str_at(&after, "run_id"));
    let db = f.database();
    let candidates = db
        .query("SELECT run_id FROM security_ingest_candidate", &[])
        .unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "old finding should remain as historical evidence"
    );
}

#[cfg(unix)]
#[test]
fn native_scanner_execution_requires_explicit_trust_and_replays() {
    use std::os::unix::fs::PermissionsExt;

    let f = Fixture::new();
    let fixture = f.temp.join("scanner-input.json");
    fs::write(
        &fixture,
        r#"{"version":"1.0.0","errors":[],"results":[{"check_id":"controlled/scan","path":"app.go","start":{"line":2}}]}"#,
    )
    .unwrap();
    let scanner = f.temp.join("fake-semgrep");
    fs::write(
        &scanner,
        format!(
            "#!/bin/sh\nout=''\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output) shift; out=\"$1\" ;;\n  esac\n  shift\ndone\ncp '{}' \"$out\"\n",
            fixture.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&scanner, fs::Permissions::from_mode(0o755)).unwrap();
    let config = f.temp.join("rules.yaml");
    fs::write(&config, "rules: []\n").unwrap();
    let report = f.temp.join("scanned.json");
    let evidence = f.temp.join("scanned.sqlite");
    let args = [
        "security",
        "scan",
        "--repo",
        f.repo.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
        "--db",
        evidence.to_str().unwrap(),
        "--engine",
        "semgrep",
        "--config",
        config.to_str().unwrap(),
        "--scanner",
        scanner.to_str().unwrap(),
        "--trusted-local",
        "true",
        "--timeout-seconds",
        "5",
    ];
    let receipt = parsed(cli(&args));
    assert_eq!(str_at(&receipt, "status"), "UNVERIFIED_STATIC_CANDIDATES");
    assert!(
        receipt
            .get("independently_replayed")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert!(report.exists());
    refused(cli(&args));

    let mut untrusted = args.to_vec();
    let trust_at = untrusted
        .iter()
        .position(|s| *s == "--trusted-local")
        .unwrap();
    untrusted[trust_at + 1] = "false";
    let other_report = f.temp.join("new-report.json");
    let report_at = untrusted.iter().position(|s| *s == "--report").unwrap();
    untrusted[report_at + 1] = other_report.to_str().unwrap();
    refused(cli(&untrusted));
    assert!(!other_report.exists());

    let mut in_source = args.to_vec();
    let forbidden_report = f.repo.join("bad-report.json");
    in_source[report_at + 1] = forbidden_report.to_str().unwrap();
    refused(cli(&in_source));
    assert!(!forbidden_report.exists());
}
