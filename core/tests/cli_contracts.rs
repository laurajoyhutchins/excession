//! Real-Git CLI contracts formerly exercised through Python's unittest runner.
//! Integration tests use Cargo's exact binary path, not target/debug guesses.
use excession::json::{self, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_excession")
}
fn fixture_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/h11-crossrepo.bundle")
}
fn run(program: &str, arguments: &[&str]) -> Output {
    Command::new(program)
        .args(arguments)
        .output()
        .expect("launch process")
}
fn ok(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn rejected(output: Output) {
    assert!(
        !output.status.success(),
        "unexpectedly accepted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty(), "rejection emitted success output");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("REFUSED:"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn cli(args: &[&str]) -> Output {
    run(binary(), args)
}
fn checked(args: &[&str]) -> Vec<u8> {
    ok(cli(args))
}
fn parsed(args: &[&str]) -> Value {
    json::parse(&checked(args)).expect("JSON response")
}
fn str_field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).unwrap().str().unwrap()
}
fn arr<'a>(value: &'a Value, name: &str) -> &'a [Value] {
    value.get(name).unwrap().array_ref().unwrap()
}
fn file(path: &Path, value: &Value) {
    fs::write(path, value.canonical()).expect("write test report");
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    commit: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn temp() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-ported-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repo = root.join("source");
        ok(run(
            "git",
            &[
                "clone",
                "-q",
                fixture_bundle().to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        ));
        let commit = String::from_utf8(ok(run(
            "git",
            &["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"],
        )))
        .unwrap()
        .trim()
        .to_owned();
        let db = root.join("index.sqlite");
        let indexed = parsed(&[
            "index",
            "--repo",
            repo.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--roots",
            "h11,tests",
        ]);
        assert_eq!(str_field(&indexed, "source_commit"), commit);
        Self {
            root,
            repo,
            db,
            commit,
        }
    }
    fn plan(&self, seed: &str, name: &str) -> (Value, PathBuf) {
        let bytes = checked(&[
            "view",
            "deps",
            "--db",
            self.db.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            seed,
        ]);
        let manifest = self.root.join(name);
        fs::write(&manifest, &bytes).unwrap();
        (json::parse(&bytes).unwrap(), manifest)
    }
    fn security(&self) -> (PathBuf, PathBuf, String) {
        let report = self.root.join("security.sarif");
        let db = self.root.join("security.sqlite");
        let sarif = r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"CodeQL"}},"results":[{"ruleId":"controlled/test-flow","locations":[{"physicalLocation":{"artifactLocation":{"uri":"h11/_connection.py"},"region":{"startLine":20}}}],"codeFlows":[{"threadFlows":[{"locations":[{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_connection.py"},"region":{"startLine":20}}}},{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_events.py"},"region":{"startLine":25}}}},{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_util.py"},"region":{"startLine":20}}}}]}]}]}]}]}"#;
        fs::write(&report, sarif).unwrap();
        let args = [
            "security",
            "ingest",
            "--repo",
            self.repo.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--engine",
            "codeql",
        ];
        let ingested = parsed(&args);
        let audited = parsed(&[
            "security",
            "audit",
            "--repo",
            self.repo.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--engine",
            "codeql",
        ]);
        assert_eq!(
            ingested.get("run_id").unwrap(),
            audited.get("run_id").unwrap()
        );
        assert!(
            audited
                .get("independently_replayed")
                .unwrap()
                .bool()
                .unwrap()
        );
        (report, db, str_field(&ingested, "run_id").to_string())
    }
}
#[test]
fn cli_help_version_and_rejection_contract() {
    let f = Fixture::temp();
    assert!(String::from_utf8_lossy(&checked(&["--help"])).contains("Excession"));
    assert!(String::from_utf8_lossy(&checked(&["view", "--help"])).contains("Excession"));
    assert!(String::from_utf8_lossy(&checked(&["--version"])).starts_with("excession "));
    // Historical positional research commands are intentionally not exposed
    // through the supported CLI.
    rejected(cli(&["research"]));
    rejected(cli(&["research", "--help"]));
    for args in [
        vec!["view", "deps", "--db", f.db.to_str().unwrap()],
        vec![
            "view",
            "deps",
            "--db",
            f.db.to_str().unwrap(),
            "--db",
            "duplicate",
        ],
        vec![
            "view",
            "replay",
            "deps",
            "--db",
            f.db.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            "h11._connection",
        ],
        vec!["view", "bad-command"],
        vec![
            "browse",
            "--repo",
            f.repo.to_str().unwrap(),
            "--manifest",
            "missing",
            "--max-bytes",
            "0",
        ],
        vec!["relgit"],
    ] {
        rejected(cli(&args));
    }
}
#[test]
fn view_dependency_plan_replays_without_legacy_command() {
    let f = Fixture::temp();
    let (plan, saved) = f.plan("h11._connection", "deps.json");
    assert_eq!(str_field(&plan, "source_commit"), f.commit);
    let args = [
        "view",
        "replay",
        "deps",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
        "--manifest",
        saved.to_str().unwrap(),
    ];
    assert_eq!(
        str_field(&parsed(&args), "view_id"),
        str_field(&plan, "view_id")
    );
    fs::write(&saved, "{}").unwrap();
    rejected(cli(&args));
}
#[test]
fn one_supported_binary_without_legacy_dispatch() {
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"excession\""));
    assert_eq!(manifest.matches("[[bin]]").count(), 1);
    rejected(cli(&["research", "json-check", "canonical.json"]));
}
#[test]
fn virtual_navigation_reads_authenticated_git_bytes() {
    let f = Fixture::temp();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    let focus = arr(&plan, "entries")
        .iter()
        .find(|x| str_field(x, "role") == "focus")
        .unwrap();
    let vpath = str_field(focus, "virtual_path");
    let root = f.repo.to_str().unwrap();
    let saved = manifest.to_str().unwrap();
    assert!(
        !arr(
            &parsed(&["view", "ls", "--repo", root, "--manifest", saved]),
            "children"
        )
        .is_empty()
    );
    let desc = parsed(&[
        "view",
        "stat",
        "--repo",
        root,
        "--manifest",
        saved,
        "--path",
        vpath,
    ]);
    assert_eq!(
        desc.get("entry").unwrap().get("blob").unwrap(),
        focus.get("blob").unwrap()
    );
    let viewed = checked(&[
        "view",
        "cat",
        "--repo",
        root,
        "--manifest",
        saved,
        "--path",
        vpath,
    ]);
    let direct = ok(run(
        "git",
        &["-C", root, "cat-file", "blob", str_field(focus, "blob")],
    ));
    assert_eq!(viewed, direct);
    rejected(cli(&[
        "view",
        "cat",
        "--repo",
        root,
        "--manifest",
        saved,
        "--path",
        "../invalid",
    ]));
}
#[test]
fn composed_selection_materialization_and_replay() {
    let f = Fixture::temp();
    let (first, a) = f.plan("h11._connection", "a.json");
    let (second, b) = f.plan("h11._headers", "b.json");
    assert_eq!(
        first.get("source_commit").unwrap(),
        second.get("source_commit").unwrap()
    );
    let root = f.repo.to_str().unwrap();
    let mount_a = f.root.join("mount-a.json");
    let mount_b = f.root.join("mount-b.json");
    fs::write(
        &mount_a,
        checked(&[
            "view",
            "mount",
            "--repo",
            root,
            "--manifest",
            a.to_str().unwrap(),
            "--namespace",
            "connection",
        ]),
    )
    .unwrap();
    fs::write(
        &mount_b,
        checked(&[
            "view",
            "mount",
            "--repo",
            root,
            "--manifest",
            b.to_str().unwrap(),
            "--namespace",
            "headers",
        ]),
    )
    .unwrap();
    let compose_args = [
        "view",
        "compose",
        "--repo",
        root,
        "--op",
        "union",
        "--left",
        mount_a.to_str().unwrap(),
        "--right",
        mount_b.to_str().unwrap(),
    ];
    let combined = parsed(&compose_args);
    assert!(arr(&combined, "entries").len() > arr(&first, "entries").len());
    let combined_file = f.root.join("combined.json");
    file(&combined_file, &combined);
    let replay = parsed(&[
        "view",
        "replay",
        "compose",
        "--repo",
        root,
        "--op",
        "union",
        "--left",
        mount_a.to_str().unwrap(),
        "--right",
        mount_b.to_str().unwrap(),
        "--output",
        combined_file.to_str().unwrap(),
    ]);
    assert_eq!(
        replay.get("view_id").unwrap(),
        combined.get("view_id").unwrap()
    );
    let selected = parsed(&[
        "view",
        "select",
        "--repo",
        root,
        "--manifest",
        combined_file.to_str().unwrap(),
        "--axis",
        "source-prefix",
        "--value",
        "h11",
    ]);
    assert!(arr(&selected, "entries").len() <= arr(&combined, "entries").len());
    let selection = f.root.join("selected.json");
    file(&selection, &selected);
    let output = f.root.join("materialized");
    let args = [
        "view",
        "materialize",
        "--repo",
        root,
        "--manifest",
        selection.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    let receipt = parsed(&args);
    let audit = parsed(&[
        "view",
        "audit",
        "--repo",
        root,
        "--manifest",
        selection.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(receipt, audit);
    rejected(cli(&args));
}
#[test]
fn security_flow_and_source_impact_can_be_inspected() {
    let f = Fixture::temp();
    let (_, db, run) = f.security();
    let root = f.repo.to_str().unwrap();
    let plan = parsed(&[
        "view",
        "security",
        "--db",
        db.to_str().unwrap(),
        "--repo",
        root,
        "--run",
        &run,
    ]);
    assert_eq!(str_field(&plan, "kind"), "security-flow");
    let manifest = f.root.join("security-view.json");
    file(&manifest, &plan);
    parsed(&[
        "view",
        "replay",
        "security",
        "--db",
        db.to_str().unwrap(),
        "--repo",
        root,
        "--run",
        &run,
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    let impact = parsed(&[
        "inspect",
        "impact",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
    ]);
    assert!(impact.get("candidate_tests").is_ok());
    assert!(
        !parsed(&[
            "inspect",
            "manifest",
            "--db",
            f.db.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            &f.commit
        ])
        .canonical()
        .is_empty()
    );
    assert!(
        !parsed(&[
            "inspect",
            "dependencies",
            "--db",
            f.db.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            &f.commit
        ])
        .canonical()
        .is_empty()
    );
}
#[test]
fn integrated_browser_does_not_materialize_source() {
    let f = Fixture::temp();
    let (_, manifest) = f.plan("h11._connection", "deps.json");
    let execute = |input: &str| {
        let mut p = Command::new(binary())
            .args([
                "browse",
                "--repo",
                f.repo.to_str().unwrap(),
                "--manifest",
                manifest.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        p.wait_with_output().unwrap()
    };
    let output = ok(execute("pwd\nls\ncd views\nls\nquit\n"));
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("views/") && text.contains("00-focus/"));
    assert!(!f.root.join("views").exists());
    rejected(execute("cd ..\n"));
}
