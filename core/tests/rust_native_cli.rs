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
            "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-qm", "nest Cargo package",
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
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report = json::parse(&output.stdout).unwrap();
    let observations = report.get("observations").unwrap().array_ref().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].get("from").unwrap().get("path").unwrap().str().unwrap(),
        "core/src/a.rs"
    );

    // A second tracked Cargo project makes auto-selection ambiguous.
    fs::create_dir_all(f.repo.join("other")).unwrap();
    fs::write(f.repo.join("other/Cargo.toml"),
        "[package]\nname=\"second\"\nversion=\"0.1.0\"\nedition=\"2024\"\n").unwrap();
    git(&f.repo, &["add", "-A"]);
    git(&f.repo, &[
        "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "second Cargo project",
    ]);
    let ambiguous = f.invoke(&[]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--source-prefix"));
    let selected = f.invoke(&["--source-prefix", "core"]);
    assert!(selected.status.success(), "{}", String::from_utf8_lossy(&selected.stderr));
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
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Rust SCIP analysis"), "{stdout}");
    assert!(stdout.contains("Cross-file symbol references: 1"), "{stdout}");
    assert!(!stdout.contains("excession.scip-observations.v1"));
}
