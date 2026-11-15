//! End-to-end Rust-native CLI boundary using a tiny deterministic SCIP producer.
//! These fixtures test orchestration and Git binding, not rust-analyzer accuracy.
#![cfg(unix)]
use excession::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    let mut mode = fs::metadata(path).unwrap().permissions();
    mode.set_mode(0o700);
    fs::set_permissions(path, mode).unwrap();
}
struct Fixture {
    temp: PathBuf,
    repo: PathBuf,
    analyzer: PathBuf,
    decoder: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}
impl Fixture {
    fn new() -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-native-rust-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = temp.join("repo with spaces");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"native-test\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(repo.join("src/a.rs"), "pub fn foo() { bar(); }\n").unwrap();
        fs::write(repo.join("src/b.rs"), "pub fn bar() {}\n").unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "pin Rust sources",
            ],
        );
        let root = repo.canonicalize().unwrap();
        let index = serde_json::json!({
            "metadata": {
                "project_root": format!("file://{}", root.display()).replace(" ", "%20"),
                "tool_info": {"name":"rust-analyzer", "version":"test"}
            },
            "documents": [
                {"relative_path":"src/a.rs","language":"rust","position_encoding":1,
                  "occurrences":[
                    {"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/foo().","symbol_roles":1},
                    {"range":[0,15,18],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":0}
                  ]},
                {"relative_path":"src/b.rs","language":"rust","position_encoding":1,
                  "occurrences":[{"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":1}]}
            ]
        });
        let payload = temp.join("decoded.json");
        fs::write(&payload, index.to_string()).unwrap();
        let analyzer = temp.join("fake-rust-analyzer");
        executable(
            &analyzer,
            "#!/bin/sh\n[ \"$1\" = scip ] || exit 4\nprintf 'native' > \"$4\"\n",
        );
        let decoder = temp.join("fake-scip");
        executable(
            &decoder,
            &format!(
                "#!/bin/sh\n[ \"$1\" = print ] && [ \"$2\" = --json ] || exit 4\ncat '{}'\n",
                payload.display()
            ),
        );
        Self {
            temp,
            repo,
            analyzer,
            decoder,
        }
    }
    fn invoke(&self, extra: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_excession"));
        cmd.args(["analyze", "rust", "--repo"])
            .arg(&self.repo)
            .args(["--trusted-local", "true", "--rust-analyzer"])
            .arg(&self.analyzer)
            .arg("--scip")
            .arg(&self.decoder)
            .arg("--json")
            .args(extra);
        cmd.output().unwrap()
    }
}
#[test]
fn native_rust_index_is_source_bound_without_an_import_parser() {
    let f = Fixture::new();
    let output = f.invoke(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json::parse(&output.stdout).unwrap();
    assert_eq!(
        report.get("schema").unwrap().str().unwrap(),
        "excession.scip-observations.v1"
    );
    assert_eq!(
        report
            .get("indexer")
            .unwrap()
            .get("name")
            .unwrap()
            .str()
            .unwrap(),
        "rust-analyzer"
    );
    assert_eq!(
        report.get("source_commit").unwrap().str().unwrap(),
        git(&f.repo, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("matched_cross_file_references")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert_eq!(
        report
            .get("observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
    assert!(!f.repo.join("index.scip").exists());
}
#[test]
fn rust_native_preflight_refuses_untrusted_and_dirty_source() {
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["analyze", "rust", "--repo"])
        .arg(&f.repo)
        .args(["--trusted-local", "false"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("trusted-local"));
    fs::write(f.repo.join("src/a.rs"), "pub fn changed() {}\n").unwrap();
    let dirty = f.invoke(&[]);
    assert!(!dirty.status.success());
    assert!(dirty.stdout.is_empty());
}
#[test]
fn native_rust_rejects_mismatched_project_root_and_source_mutation() {
    let f = Fixture::new();
    let version = git(&f.repo, &["rev-parse", "HEAD"]);
    let mismatched = f.invoke(&["--rev", &format!("{version}^")]);
    assert!(!mismatched.status.success());
    assert!(mismatched.stdout.is_empty());
    executable(
        &f.analyzer,
        &format!(
            "#!/bin/sh\nprintf 'native' > \"$4\"\nprintf 'changed' > '{}/src/a.rs'\n",
            f.repo.display()
        ),
    );
    let changed = f.invoke(&[]);
    assert!(!changed.status.success());
    assert!(changed.stdout.is_empty());
}

#[test]
fn native_rust_auto_detects_sole_nested_cargo_manifest() {
    let f = Fixture::new();
    fs::create_dir_all(f.repo.join("core")).unwrap();
    fs::rename(f.repo.join("Cargo.toml"), f.repo.join("core/Cargo.toml")).unwrap();
    fs::rename(f.repo.join("src"), f.repo.join("core/src")).unwrap();
    git(&f.repo, &["add", "-A"]);
    git(
        &f.repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "nest Cargo package",
        ],
    );
    let index_path = f.temp.join("decoded.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    let new_root = f.repo.canonicalize().unwrap().join("core");
    index["metadata"]["project_root"] =
        serde_json::json!(format!("file://{}", new_root.display()).replace(" ", "%20"));
    fs::write(index_path, index.to_string()).unwrap();

    let output = f.invoke(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json::parse(&output.stdout).unwrap();
    let observations = report.get("observations").unwrap().array_ref().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0]
            .get("from")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap(),
        "core/src/a.rs"
    );

    // A second tracked Cargo project makes auto-selection ambiguous.
    fs::create_dir_all(f.repo.join("other")).unwrap();
    fs::write(
        f.repo.join("other/Cargo.toml"),
        "[package]\nname=\"second\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    git(&f.repo, &["add", "-A"]);
    git(
        &f.repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "second Cargo project",
        ],
    );
    let ambiguous = f.invoke(&[]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--source-prefix"));
    let selected = f.invoke(&["--source-prefix", "core"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
}

#[test]
fn rust_native_defaults_to_human_summary_and_accepts_bare_trust_flag() {
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .current_dir(&f.repo)
        .args(["analyze", "rust", "--trusted-local", "--rust-analyzer"])
        .arg(&f.analyzer)
        .arg("--scip")
        .arg(&f.decoder)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Rust SCIP analysis"), "{stdout}");
    assert!(
        stdout.contains("Cross-file symbol references: 1"),
        "{stdout}"
    );
    assert!(!stdout.contains("excession.scip-observations.v1"));
}

#[test]
fn shared_native_rust_report_replays_real_scip_edges_and_refuses_forged_git_blobs() {
    let f = Fixture::new();
    let exe = env!("CARGO_BIN_EXE_excession");
    let output = Command::new(exe)
        .args(["analyze", "native", "--repo"])
        .arg(&f.repo)
        .args([
            "--language",
            "rust",
            "--source",
            "src/a.rs",
            "--trusted-local",
            "true",
            "--rust-analyzer",
        ])
        .arg(&f.analyzer)
        .arg("--scip")
        .arg(&f.decoder)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json::parse(&output.stdout).unwrap();
    let field = |key: &str| report.get(key).unwrap().str().unwrap().to_owned();
    assert_eq!(field("schema"), "excession.native-analysis.v1");
    assert_eq!(field("language"), "rust");
    assert_eq!(field("producer"), "rust-analyzer-scip");
    assert_eq!(
        field("scope"),
        "SCIP_WORKSPACE_REFERENCES_NOT_BUILD_VERIFIED"
    );
    assert_eq!(field("execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
    assert!(report.content_id_matches("report_id").unwrap());
    assert_eq!(report.get("inputs").unwrap().array_ref().unwrap().len(), 1);
    let nested = report.get("observation").unwrap();
    assert_eq!(
        nested.get("schema").unwrap().str().unwrap(),
        "excession.scip-observations.v1"
    );
    assert_eq!(
        nested
            .get("observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );

    let path = f.temp.join("native-rust.json");
    fs::write(&path, report.canonical()).unwrap();
    let db = f.temp.join("native-rust.sqlite");
    let import = Command::new(exe)
        .args(["evidence", "native-import", "--repo"])
        .arg(&f.repo)
        .args(["--rev", "HEAD", "--report"])
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        import.status.success(),
        "{}",
        String::from_utf8_lossy(&import.stderr)
    );
    let receipt = json::parse(&import.stdout).unwrap();
    let snapshot = receipt.get("snapshot_id").unwrap().str().unwrap();
    let store = excession::sqlite::Db::open(&db).unwrap();
    let plan = excession::observed_relations::default_plan(&store, &f.repo, "HEAD").unwrap();
    assert_eq!(plan.native.len(), 1);
    let observed = excession::observed_relations::read_plan(&store, &f.repo, "HEAD", &plan).unwrap();
    assert_eq!(observed.relationships.len(), 1);
    assert_eq!(observed.relationships[0].get("kind").unwrap().str().unwrap(), "SCIP_REFERENCE");
    assert_eq!(observed.coverage.len(), 1);
    let audit = Command::new(exe)
        .args(["evidence", "native-store-audit", "--repo"])
        .arg(&f.repo)
        .args(["--rev", "HEAD", "--snapshot", snapshot, "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    assert_eq!(
        json::parse(&audit.stdout)
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "REPLAYED_GIT_BOUND_NATIVE_OBSERVATIONS"
    );

    // A self-consistent rehashed report must still fail replay if a producer
    // fabricates a different Git blob for a SCIP reference endpoint.
    let mut forged_native = nested.clone();
    let mut edges = forged_native
        .get("observations")
        .unwrap()
        .array_ref()
        .unwrap()
        .to_vec();
    let mut edge = edges[0].clone();
    let mut to = edge.get("to").unwrap().clone();
    to.entries_mut().unwrap().insert(
        "blob".into(),
        excession::json::Value::string("0".repeat(40)),
    );
    edge.entries_mut().unwrap().insert("to".into(), to);
    edges[0] = edge;
    forged_native
        .entries_mut()
        .unwrap()
        .insert("observations".into(), excession::json::Value::array(edges));
    forged_native.entries_mut().unwrap().remove("report_id");
    let forged_native = forged_native.stamp_id("report_id").unwrap();
    let mut forged = report.clone();
    forged
        .entries_mut()
        .unwrap()
        .insert("observation".into(), forged_native);
    forged.entries_mut().unwrap().remove("report_id");
    let forged = forged.stamp_id("report_id").unwrap();
    fs::write(&path, forged.canonical()).unwrap();
    let rejected = Command::new(exe)
        .args(["evidence", "native-import", "--repo"])
        .arg(&f.repo)
        .args(["--rev", "HEAD", "--report"])
        .arg(&path)
        .arg("--db")
        .arg(&db)
        .output()
        .unwrap();
    assert!(!rejected.status.success(), "forged Git blob was accepted");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("pinned Git blob"));
}

#[test]
fn shared_rust_cli_rejects_inapplicable_compiler_options_before_launch() {
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args([
            "analyze",
            "native",
            "--language",
            "rust",
            "--source",
            "src/a.rs",
            "--compiler",
            "clang",
            "--trusted-local",
        ])
        .arg("--repo")
        .arg(&f.repo)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Rust SCIP does not use"));
}
