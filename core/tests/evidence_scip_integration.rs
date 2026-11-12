//! SCIP evidence end-to-end: existing Git authority, immutable SQLite and CLI.
use excession::json::{self, Value};
use excession::sqlite::{Db, Param};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}
fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn refusal(output: Output) {
    assert!(!output.status.success(), "unexpectedly accepted");
    assert!(output.stdout.is_empty());
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(output: Output) -> Value {
    json::parse(&success(output)).unwrap()
}
fn s<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).unwrap().str().unwrap()
}
fn index() -> Value {
    json::parse(
        br#"{
          "metadata":{"tool_info":{"name":"fixture-scip","version":"1.0"}},
          "documents":[
            {"relative_path":"src/a.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/foo().","symbol_roles":1},
                {"range":[0,15,18],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":0}
              ]},
            {"relative_path":"src/b.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":1}
              ]}
          ]
        }"#,
    )
    .unwrap()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    input: PathBuf,
    commit: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-scip-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        let repo_path = repo.to_str().unwrap();
        success(run("git", &["-C", repo_path, "init", "-q"]));
        fs::write(repo.join("src/a.rs"), "pub fn foo() { bar(); }\n").unwrap();
        fs::write(repo.join("src/b.rs"), "pub fn bar() {}\n").unwrap();
        success(run("git", &["-C", repo_path, "add", "."]));
        success(run(
            "git",
            &[
                "-C",
                repo_path,
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "pin sources",
            ],
        ));
        let commit =
            String::from_utf8(success(run("git", &["-C", repo_path, "rev-parse", "HEAD"])))
                .unwrap()
                .trim()
                .to_owned();
        let input = root.join("scip-decoded.json");
        fs::write(&input, index().canonical()).unwrap();
        let db = root.join("evidence.sqlite");
        Self {
            root,
            repo,
            db,
            input,
            commit,
        }
    }
    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn db(&self) -> &str {
        self.db.to_str().unwrap()
    }
    fn input(&self) -> &str {
        self.input.to_str().unwrap()
    }
    fn import(&self) -> Output {
        cli(&[
            "refactor",
            "scip-ingest",
            "--repo",
            self.repo(),
            "--rev",
            &self.commit,
            "--index",
            self.input(),
            "--db",
            self.db(),
        ])
    }
    fn inspect(&self, snapshot: &str, focus: Option<&str>) -> Output {
        let mut args = vec![
            "inspect",
            "scip-evidence",
            "--db",
            self.db(),
            "--snapshot",
            snapshot,
        ];
        if let Some(path) = focus {
            args.extend(["--focus", path]);
        }
        cli(&args)
    }
    fn audit(&self, snapshot: &str, rev: &str) -> Output {
        cli(&[
            "refactor",
            "scip-store-audit",
            "--repo",
            self.repo(),
            "--rev",
            rev,
            "--db",
            self.db(),
            "--snapshot",
            snapshot,
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn observed_reference_survives_git_bound_import_explain_and_replay() {
    let f = Fixture::new();
    // The existing reciprocal/cross-language *hypothesis* report is empty.
    // This new evidence path must still retain every matched cross-file ref.
    let prior = data(cli(&[
        "refactor",
        "scip-suggest",
        "--repo",
        f.repo(),
        "--rev",
        &f.commit,
        "--index",
        f.input(),
    ]));
    assert_eq!(
        prior.get("candidates").unwrap().array_ref().unwrap().len(),
        0
    );
    let imported = data(f.import());
    assert_eq!(s(&imported, "source_commit"), f.commit);
    assert_eq!(
        s(&imported, "source_binding"),
        "GIT_OBJECTS_REPLAYED_ON_IMPORT"
    );
    assert_eq!(
        imported
            .get("matched_cross_file_references")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    let id = s(&imported, "snapshot_id");
    assert_eq!(s(&data(f.import()), "snapshot_id"), id);
    let from = data(f.inspect(id, Some("src/a.rs")));
    assert_eq!(
        from.get("matched_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert!(!from.get("absence_proven").unwrap().bool().unwrap());
    let obs = &from.get("observations").unwrap().array_ref().unwrap()[0];
    assert_eq!(s(obs, "direction"), "OUTGOING");
    assert_eq!(
        s(obs.get("payload").unwrap(), "relation"),
        "SCIP_SYMBOL_REFERENCE"
    );
    assert_eq!(
        s(obs.get("payload").unwrap().get("to").unwrap(), "path"),
        "src/b.rs"
    );
    let to = data(f.inspect(id, Some("src/b.rs")));
    assert_eq!(
        s(
            &to.get("observations").unwrap().array_ref().unwrap()[0],
            "direction"
        ),
        "INCOMING"
    );
    let verified = data(f.audit(id, &f.commit));
    assert_eq!(
        s(&verified, "status"),
        "REPLAYED_GIT_BOUND_SCIP_OBSERVATIONS"
    );
    assert!(
        !verified
            .get("indexer_independently_executed")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert!(
        !verified
            .get("symbol_semantics_independently_verified")
            .unwrap()
            .bool()
            .unwrap()
    );
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_snapshot_v1", &[])
            .unwrap(),
        "1"
    );
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_receipt_v1", &[])
            .unwrap(),
        "1"
    );
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_observation_v1", &[])
            .unwrap(),
        "2"
    );
}

#[test]
fn missing_target_remains_unknown_never_proves_absence() {
    let f = Fixture::new();
    let mut changed = index();
    let records = changed.get("documents").unwrap().array_ref().unwrap();
    let mut modified = records.to_vec();
    let a = modified[0]
        .get("occurrences")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    let mut refs = a;
    refs[1].entries_mut().unwrap().insert(
        "symbol".into(),
        Value::string("scip rust cargo demo 0.1.0 demo/unresolved()."),
    );
    modified[0]
        .entries_mut()
        .unwrap()
        .insert("occurrences".into(), Value::array(refs));
    changed
        .entries_mut()
        .unwrap()
        .insert("documents".into(), Value::array(modified));
    fs::write(&f.input, changed.canonical()).unwrap();
    let imported = data(f.import());
    assert_eq!(
        imported
            .get("unresolved_references")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    let report = data(f.inspect(s(&imported, "snapshot_id"), Some("src/a.rs")));
    assert_eq!(
        report
            .get("matched_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
    assert!(!report.get("absence_proven").unwrap().bool().unwrap());
    assert_eq!(s(report.get("coverage").unwrap(), "status"), "PARTIAL");
    data(f.audit(s(&imported, "snapshot_id"), &f.commit));
}

#[test]
fn forged_input_path_or_embedded_text_is_refused() {
    let f = Fixture::new();
    let mut wrong = index();
    let mut docs = wrong
        .get("documents")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    docs[0]
        .entries_mut()
        .unwrap()
        .insert("relative_path".into(), Value::string("../escape.rs"));
    wrong
        .entries_mut()
        .unwrap()
        .insert("documents".into(), Value::array(docs));
    fs::write(&f.input, wrong.canonical()).unwrap();
    refusal(f.import());
    let mut wrong = index();
    let mut docs = wrong
        .get("documents")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    docs[0]
        .entries_mut()
        .unwrap()
        .insert("text".into(), Value::string("forged source"));
    wrong
        .entries_mut()
        .unwrap()
        .insert("documents".into(), Value::array(docs));
    fs::write(&f.input, wrong.canonical()).unwrap();
    refusal(f.import());
}

#[test]
fn source_change_or_sql_row_tampering_invalidates_replay() {
    let f = Fixture::new();
    let imported = data(f.import());
    let id = s(&imported, "snapshot_id");
    fs::write(f.repo.join("src/a.rs"), "pub fn foo() { hacked(); }\n").unwrap();
    success(run("git", &["-C", f.repo(), "add", "."]));
    success(run(
        "git",
        &[
            "-C",
            f.repo(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "change source",
        ],
    ));
    refusal(f.audit(id, "HEAD"));
    data(f.audit(id, &f.commit));
    let db = Db::open(&f.db).unwrap();
    db.execute(
        "UPDATE evidence_receipt_v1 SET document=? WHERE snapshot_id=?",
        &[Param::Text("{}"), Param::Text(id)],
    )
    .unwrap();
    refusal(f.inspect(id, Some("src/a.rs")));
    refusal(f.audit(id, &f.commit));
}

#[test]
fn missing_or_forged_extra_observation_refused() {
    for mutation in [
        "DELETE FROM evidence_observation_v1",
        "INSERT INTO evidence_observation_v1(snapshot_id,observation_id,receipt_id,document) SELECT snapshot_id,'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',receipt_id,document FROM evidence_observation_v1 LIMIT 1",
    ] {
        let f = Fixture::new();
        let imported = data(f.import());
        let id = s(&imported, "snapshot_id");
        let db = Db::open(&f.db).unwrap();
        db.exec(mutation).unwrap();
        refusal(f.inspect(id, None));
        refusal(f.audit(id, &f.commit));
    }
}

#[test]
fn authenticated_inspection_checks_pinned_source_in_one_command() {
    let f = Fixture::new();
    let imported = data(f.import());
    let id = s(&imported, "snapshot_id");
    let answer = data(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--focus",
        "src/a.rs",
        "--repo",
        f.repo(),
        "--rev",
        &f.commit,
    ]));
    assert_eq!(
        s(&answer, "source_authentication"),
        "GIT_OBJECTS_REPLAYED_FOR_QUERY"
    );
    assert_eq!(
        answer
            .get("matched_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    refusal(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--rev",
        &f.commit,
    ]));
    refusal(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--source-prefix",
        "src",
    ]));
    fs::write(f.repo.join("src/a.rs"), "pub fn foo() { altered(); }\n").unwrap();
    success(run("git", &["-C", f.repo(), "add", "."]));
    success(run(
        "git",
        &[
            "-C",
            f.repo(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "changed",
        ],
    ));
    refusal(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--repo",
        f.repo(),
        "--rev",
        "HEAD",
    ]));
}

#[test]
fn paginated_scip_references_are_complete_and_scope_bound() {
    use std::collections::BTreeSet;
    let mut f = Fixture::new();
    let mut body = String::from("pub fn foo() { bar(); }\n");
    for _ in 0..300 {
        body.push_str("bar();\n");
    }
    fs::write(f.repo.join("src/a.rs"), body).unwrap();
    success(run("git", &["-C", f.repo(), "add", "."]));
    success(run(
        "git",
        &[
            "-C",
            f.repo(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "many lines",
        ],
    ));
    f.commit = String::from_utf8(success(run("git", &["-C", f.repo(), "rev-parse", "HEAD"])))
        .unwrap()
        .trim()
        .to_owned();

    let mut source_index = index();
    let documents = source_index
        .get("documents")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    let mut documents = documents;
    let occ = documents[0]
        .get("occurrences")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    let mut occ = occ;
    for line in 1..=300 {
        let serialized = format!(
            "{{\"range\":[{line},0,3],\"symbol\":\"scip rust cargo demo 0.1.0 demo/bar().\",\"symbol_roles\":0}}"
        );
        occ.push(json::parse(serialized.as_bytes()).unwrap());
    }
    documents[0]
        .entries_mut()
        .unwrap()
        .insert("occurrences".into(), Value::array(occ));
    source_index
        .entries_mut()
        .unwrap()
        .insert("documents".into(), Value::array(documents));
    fs::write(&f.input, source_index.canonical()).unwrap();
    let imported = data(f.import());
    let id = s(&imported, "snapshot_id");
    assert_eq!(
        imported
            .get("matched_cross_file_references")
            .unwrap()
            .integer()
            .unwrap(),
        301
    );
    let first = data(f.inspect(id, Some("src/a.rs")));
    assert!(first.get("truncated").unwrap().bool().unwrap());
    assert_eq!(
        first
            .get("returned_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        256
    );
    let cursor = s(&first, "next_cursor");
    let second = data(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--focus",
        "src/a.rs",
        "--after",
        cursor,
    ]));
    assert_eq!(
        second
            .get("returned_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        45
    );
    assert!(!second.get("truncated").unwrap().bool().unwrap());
    assert_eq!(second.get("next_cursor").unwrap(), &Value::Null);
    assert_eq!(
        second
            .get("matched_observation_count")
            .unwrap()
            .integer()
            .unwrap(),
        301
    );
    assert!(!second.get("absence_proven").unwrap().bool().unwrap());
    let mut unique = BTreeSet::new();
    for page in [&first, &second] {
        for item in page.get("observations").unwrap().array_ref().unwrap() {
            assert!(unique.insert(s(item, "observation_id")));
        }
    }
    assert_eq!(unique.len(), 301);
    refusal(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--focus",
        "src/unindexed.rs",
        "--after",
        cursor,
    ]));
    refusal(cli(&[
        "inspect",
        "scip-evidence",
        "--db",
        f.db(),
        "--snapshot",
        id,
        "--after",
        "bad-cursor",
    ]));
}
