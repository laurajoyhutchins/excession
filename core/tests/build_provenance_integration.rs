//! Independent real Git and rustc fixture for source/artifact evidence.
use excession::digest;
use excession::json::Value;
use excession::sqlite::Db;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .output()
        .expect("spawn subprocess")
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
    assert!(out.stdout.is_empty(), "failed CLI must not claim result");
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn parsed(out: Output) -> Value {
    excession::json::parse(&success(out)).unwrap()
}
fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).unwrap().str().unwrap()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    artifact: PathBuf,
    db: PathBuf,
    commit: String,
    toolchain: String,
    configuration: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-build-proof-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        success(run("git", &["-C", repo.to_str().unwrap(), "init", "-q"]));
        fs::write(
            repo.join("src/main.rs"),
            "fn main() { println!(\"pinned\"); }\n",
        )
        .unwrap();
        success(run("git", &["-C", repo.to_str().unwrap(), "add", "."]));
        success(run(
            "git",
            &[
                "-C",
                repo.to_str().unwrap(),
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "source",
            ],
        ));
        let commit = String::from_utf8(success(run(
            "git",
            &["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"],
        )))
        .unwrap()
        .trim()
        .to_owned();
        let artifact = root.join("application");
        success(run(
            "rustc",
            &[
                "--edition=2024",
                "-C",
                "debuginfo=0",
                repo.join("src/main.rs").to_str().unwrap(),
                "-o",
                artifact.to_str().unwrap(),
            ],
        ));
        let toolchain = digest::sha256(&success(run("rustc", &["--version"])));
        let configuration = digest::sha256(b"fixture rustc edition=2024 debuginfo=0");
        let db = root.join("evidence.sqlite");
        Self {
            root,
            repo,
            artifact,
            db,
            commit,
            toolchain,
            configuration,
        }
    }
    fn db(&self) -> &str {
        self.db.to_str().unwrap()
    }
    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn artifact(&self) -> &str {
        self.artifact.to_str().unwrap()
    }
    fn record(&self, sources: &str, tc: &str) -> Output {
        cli(&[
            "evidence",
            "build-record",
            "--db",
            self.db(),
            "--repo",
            self.repo(),
            "--rev",
            &self.commit,
            "--artifact",
            self.artifact(),
            "--sources",
            sources,
            "--toolchain",
            tc,
            "--configuration",
            &self.configuration,
            "--representation",
            "EXECUTABLE",
        ])
    }
    fn audit(&self, rev: &str, snapshot: &str) -> Output {
        cli(&[
            "inspect",
            "build-provenance",
            "--db",
            self.db(),
            "--repo",
            self.repo(),
            "--rev",
            rev,
            "--artifact",
            self.artifact(),
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
fn compiled_rust_artifact_has_replayable_provenance_but_no_execution_proof() {
    let f = Fixture::new();
    let record = parsed(f.record("src/main.rs", &f.toolchain));
    assert_eq!(text(&record, "source_artifact_relation"), "NOT_ESTABLISHED");
    assert_eq!(
        text(&record, "status"),
        "GIT_INPUTS_AND_ARTIFACT_BYTES_REPLAYED"
    );
    assert!(
        record
            .get("artifact_bytes_verified")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert!(record.get("git_inputs_verified").unwrap().bool().unwrap());
    assert!(
        !record
            .get("build_execution_verified")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert_eq!(
        text(record.get("artifact").unwrap(), "sha256"),
        digest::sha256(&fs::read(&f.artifact).unwrap())
    );
    let snapshot = text(&record, "snapshot_id");
    let replay = parsed(f.audit(&f.commit, snapshot));
    assert_eq!(record, replay);
    assert_eq!(
        text(
            &parsed(f.record("src/main.rs", &f.toolchain)),
            "snapshot_id"
        ),
        snapshot
    );

    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/build-provenance.v1.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let output: serde_json::Value = serde_json::from_str(&record.canonical()).unwrap();
    assert!(validator.is_valid(&output), "actual report violates schema");

    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_snapshot_v1", &[])
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
fn changed_artifact_and_untrusted_recipe_cannot_be_upgraded() {
    let f = Fixture::new();
    let record = parsed(f.record("src/main.rs", &f.toolchain));
    let snapshot = text(&record, "snapshot_id");
    refused(f.record("src/main.rs,src/main.rs", &f.toolchain));
    refused(f.record("src/missing.rs", &f.toolchain));
    refused(f.record("src/main.rs", "not-a-digest"));
    fs::OpenOptions::new()
        .append(true)
        .open(&f.artifact)
        .unwrap()
        .write_all(b"tampered")
        .unwrap();
    refused(f.audit(&f.commit, snapshot));
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_snapshot_v1", &[])
            .unwrap(),
        "1"
    );
}
#[test]
fn wrong_revision_or_modified_stored_observation_refuses() {
    let f = Fixture::new();
    let record = parsed(f.record("src/main.rs", &f.toolchain));
    let snapshot = text(&record, "snapshot_id");
    fs::write(
        f.repo.join("src/main.rs"),
        "fn main(){println!(\"different\");}\n",
    )
    .unwrap();
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
    refused(f.audit("HEAD", snapshot));
    parsed(f.audit(&f.commit, snapshot));
    let db = Db::open(&f.db).unwrap();
    db.exec("UPDATE evidence_observation_v1 SET document='{}'")
        .unwrap();
    refused(f.audit(&f.commit, snapshot));
}
