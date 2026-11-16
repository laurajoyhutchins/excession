//! A real multi-project compiler matrix: Cargo, TypeScript, and mypy
//! remain separate predicates, and every Git declaration is replayable.
use excession::json::{self, Value};
use excession::sqlite::Db;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(name: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(name)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}
fn ok(name: &str, args: &[&str], dir: &Path) -> String {
    let result = run(name, args, dir);
    assert!(
        result.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_excession"))
}
fn result(out: Output, success: bool) -> Value {
    assert_eq!(
        out.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = json::parse(&out.stdout).unwrap();
    assert_eq!(
        value.get("status").unwrap().str().unwrap(),
        if success { "PASS" } else { "FAIL" }
    );
    assert_eq!(
        value
            .get("artifact_build_provenance")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert_eq!(value.get("rea").unwrap().str().unwrap(), "NOT_ESTABLISHED");
    assert_eq!(
        value.get("execution_authority").unwrap().str().unwrap(),
        "TRUSTED_LOCAL_NOT_ATTESTED"
    );
    value
}
struct Fixture {
    temp: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    commit: String,
}
impl Fixture {
    fn new(manifest: &str) -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-matrix-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = temp.join("repo");
        fs::create_dir_all(&repo).unwrap();
        ok("git", &["init", "-q"], &repo);
        for (path, content) in [
            (
                "packages/a/Cargo.toml",
                "[package]\nname = \"excession-matrix-a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("packages/a/src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            (
                "packages/b/Cargo.toml",
                "[package]\nname = \"excession-matrix-b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("packages/b/src/lib.rs", "pub fn double() -> u32 { 84 }\n"),
            (
                "web/tsconfig.json",
                "{\"compilerOptions\":{\"strict\":true,\"target\":\"ES2022\"},\"include\":[\"src/**/*.ts\"]}\n",
            ),
            ("web/src/main.ts", "export const answer: number = 42;\n"),
            ("mypy.ini", "[mypy]\nstrict = True\n"),
            ("py/__init__.py", ""),
            ("py/mathlib.py", "def answer() -> int:\n    return 42\n"),
            ("checks.json", manifest),
        ] {
            let location = repo.join(path);
            fs::create_dir_all(location.parent().unwrap()).unwrap();
            fs::write(location, content).unwrap();
        }
        for name in ["a", "b"] {
            ok(
                "cargo",
                &[
                    "generate-lockfile",
                    "--offline",
                    "--manifest-path",
                    repo.join("packages")
                        .join(name)
                        .join("Cargo.toml")
                        .to_str()
                        .unwrap(),
                ],
                &repo,
            );
        }
        ok("git", &["add", "."], &repo);
        ok(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
            &repo,
        );
        let commit = ok("git", &["rev-parse", "HEAD"], &repo);
        Self {
            db: temp.join("evidence.sqlite"),
            temp,
            repo,
            commit,
        }
    }
    fn matrix(&self, ts_tool: Option<&Path>, trusted: bool) -> Output {
        let mut command = cli();
        command
            .args(["check", "matrix", "--repo"])
            .arg(&self.repo)
            .args(["--rev", &self.commit, "--manifest", "checks.json", "--db"])
            .arg(&self.db)
            .args(["--trusted-local", if trusted { "true" } else { "false" }]);
        if let Some(tool) = ts_tool {
            command.arg("--typescript-tool").arg(tool);
        }
        command.output().unwrap()
    }
    fn audit(&self, snapshot: &str, rev: &str) -> Output {
        cli()
            .args(["check", "matrix-audit", "--repo"])
            .arg(&self.repo)
            .args(["--rev", rev, "--db"])
            .arg(&self.db)
            .args(["--snapshot", snapshot])
            .output()
            .unwrap()
    }
    fn change(&mut self, file: &str, content: &str) {
        fs::write(self.repo.join(file), content).unwrap();
        ok("git", &["add", file], &self.repo);
        ok(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "mutation",
            ],
            &self.repo,
        );
        self.commit = ok("git", &["rev-parse", "HEAD"], &self.repo);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}
const TWO_CARGO: &str = r#"{
  "schema": "excession.compiler-matrix-config.v1",
  "checks": [
    {"language":"rust","configuration":"packages/a/Cargo.toml",
     "source_prefix":null,
     "declared_inputs":["packages/a/Cargo.lock","packages/a/src/lib.rs"]},
    {"language":"rust","configuration":"packages/b/Cargo.toml",
     "source_prefix":null,
     "declared_inputs":["packages/b/Cargo.lock","packages/b/src/lib.rs"]}
  ]
}"#;
const THREE_LANGUAGES: &str = r#"{
  "schema":"excession.compiler-matrix-config.v1",
  "checks":[
    {"language":"rust","configuration":"packages/a/Cargo.toml","source_prefix":null,
     "declared_inputs":["packages/a/Cargo.lock"]},
    {"language":"typescript","configuration":"web/tsconfig.json","source_prefix":null,
     "declared_inputs":["web/src/main.ts"]},
    {"language":"python","configuration":"mypy.ini","source_prefix":"py",
     "declared_inputs":["py/mathlib.py"]}
  ]
}"#;

#[test]
fn two_independent_cargo_projects_store_and_audit_one_exact_git_matrix() {
    let f = Fixture::new(TWO_CARGO);
    let report = result(f.matrix(None, true), true);
    assert_eq!(
        report.get("schema").unwrap().str().unwrap(),
        "excession.compiler-matrix.v1"
    );
    assert_eq!(report.get("checks").unwrap().array_ref().unwrap().len(), 2);
    assert_eq!(
        report.get("source_commit").unwrap().str().unwrap(),
        f.commit
    );
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("configuration_completeness")
            .unwrap()
            .str()
            .unwrap(),
        "USER_DECLARATION_NOT_EXHAUSTIVE"
    );
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("declared_inputs")
            .unwrap()
            .str()
            .unwrap(),
        "GIT_AUTHENTICATED_NOT_COMPILER_MEMBERSHIP"
    );
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("dependency_closure")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert_eq!(
        report.get("checks").unwrap().array_ref().unwrap()[0]
            .get("declared_inputs")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        2
    );
    let id = report.get("snapshot_id").unwrap().str().unwrap();
    let audited = f.audit(id, "HEAD");
    assert!(
        audited.status.success(),
        "{}",
        String::from_utf8_lossy(&audited.stderr)
    );
    assert_eq!(report, json::parse(&audited.stdout).unwrap());
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_receipt_v1", &[])
            .unwrap(),
        "3"
    );
    assert!(!f.repo.join("packages/a/target").exists());
    assert!(!f.repo.join("packages/b/target").exists());
}

#[test]
fn matrix_failure_keeps_both_children_and_does_not_become_build_attestation() {
    let mut f = Fixture::new(TWO_CARGO);
    f.change(
        "packages/a/src/lib.rs",
        "pub fn answer() -> u32 { \"bad\" }\n",
    );
    let report = result(f.matrix(None, true), false);
    let checks = report.get("checks").unwrap().array_ref().unwrap();
    assert_eq!(checks[0].get("status").unwrap().str().unwrap(), "FAIL");
    assert_eq!(checks[1].get("status").unwrap().str().unwrap(), "PASS");
    assert_eq!(
        json::parse(
            &f.audit(report.get("snapshot_id").unwrap().str().unwrap(), "HEAD")
                .stdout
        )
        .unwrap(),
        report
    );
}

#[test]
fn matrix_refuses_untrusted_and_unbound_dependency_before_running_any_tool() {
    let f = Fixture::new(TWO_CARGO);
    let untrusted = f.matrix(None, false);
    assert!(!untrusted.status.success());
    assert!(untrusted.stdout.is_empty());
    fs::write(f.repo.join("packages/b/Cargo.lock"), "modified\n").unwrap();
    let dirty = f.matrix(None, true);
    assert!(!dirty.status.success());
    assert!(dirty.stdout.is_empty());
    let db = Db::open(&f.db).unwrap();
    assert!(
        db.scalar(
            "SELECT count(*) FROM sqlite_master WHERE name='evidence_receipt_v1'",
            &[]
        )
        .unwrap()
            == "0"
    );
}

#[test]
fn old_git_revision_replays_but_new_revision_and_tampered_observation_refuse() {
    let mut f = Fixture::new(TWO_CARGO);
    let report = result(f.matrix(None, true), true);
    let snapshot = report.get("snapshot_id").unwrap().str().unwrap().to_owned();
    let original = f.commit.clone();
    f.change("packages/a/src/lib.rs", "pub fn answer() -> u32 { 43 }\n");
    assert!(!f.audit(&snapshot, "HEAD").status.success());
    assert!(f.audit(&snapshot, &original).status.success());
    let db = Db::open(&f.db).unwrap();
    db.exec("UPDATE evidence_observation_v1 SET document='{}'")
        .unwrap();
    assert!(!f.audit(&snapshot, &original).status.success());
}

#[test]
fn full_rust_typescript_mypy_matrix_in_its_native_ci_environment() {
    let ts = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("node_modules/.bin/tsc");
    if !ts.is_file() || Command::new("mypy").arg("--version").output().is_err() {
        return;
    }
    let f = Fixture::new(THREE_LANGUAGES);
    let report = result(f.matrix(Some(&ts.canonicalize().unwrap()), true), true);
    let checks = report.get("checks").unwrap().array_ref().unwrap();
    assert_eq!(checks.len(), 3);
    assert_eq!(
        checks[1].get("language").unwrap().str().unwrap(),
        "typescript"
    );
    assert_eq!(checks[2].get("language").unwrap().str().unwrap(), "python");
    assert!(
        f.audit(report.get("snapshot_id").unwrap().str().unwrap(), "HEAD")
            .status
            .success()
    );
}

#[test]
fn equivalent_python_scopes_are_not_double_counted() {
    let manifest = r#"{"schema":"excession.compiler-matrix-config.v1","checks":[
      {"language":"python","configuration":"mypy.ini","source_prefix":null,
       "declared_inputs":["py/mathlib.py"]},
      {"language":"python","configuration":"mypy.ini","source_prefix":".",
       "declared_inputs":["py/mathlib.py"]}
    ]}"#;
    let f = Fixture::new(manifest);
    let output = f.matrix(None, true);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate"));
}

#[test]
fn non_python_check_refuses_an_explicit_dot_scope() {
    let manifest = r#"{"schema":"excession.compiler-matrix-config.v1","checks":[
      {"language":"rust","configuration":"packages/a/Cargo.toml","source_prefix":".",
       "declared_inputs":["packages/a/Cargo.lock"]}
    ]}"#;
    let f = Fixture::new(manifest);
    let output = f.matrix(None, true);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("source_prefix is only supported for Python")
    );
}

#[test]
fn a_large_tracked_lockfile_is_accepted_without_inventing_dependency_coverage() {
    let manifest = r#"{"schema":"excession.compiler-matrix-config.v1","checks":[
      {"language":"rust","configuration":"packages/a/Cargo.toml","source_prefix":null,
       "declared_inputs":["large.lock"]}
    ]}"#;
    let mut f = Fixture::new(manifest);
    f.change("large.lock", &"x".repeat(1_200_000));
    let report = result(f.matrix(None, true), true);
    let input = &report.get("checks").unwrap().array_ref().unwrap()[0]
        .get("declared_inputs")
        .unwrap()
        .array_ref()
        .unwrap()[0];
    assert_eq!(input.get("path").unwrap().str().unwrap(), "large.lock");
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("dependency_closure")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert!(
        f.audit(report.get("snapshot_id").unwrap().str().unwrap(), "HEAD")
            .status
            .success()
    );
}

#[test]
fn unrelated_filename_ending_in_cargo_toml_is_refused() {
    let manifest = r#"{"schema":"excession.compiler-matrix-config.v1","checks":[
      {"language":"rust","configuration":"packages/a/fakeCargo.toml","source_prefix":null,
       "declared_inputs":[]}
    ]}"#;
    let mut f = Fixture::new(manifest);
    f.change(
        "packages/a/fakeCargo.toml",
        "[package]\nname=\"fake\"\nversion=\"0.1.0\"\n",
    );
    let output = f.matrix(None, true);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires a Cargo.toml configuration")
    );
}

#[test]
fn malformed_manifest_fails_before_child_execution() {
    let manifest = r#"{"schema":"excession.compiler-matrix-config.v1","checks":[
      {"language":"rust","configuration":"packages/a/Cargo.toml","source_prefix":null,
       "declared_inputs":["packages/a/Cargo.lock"]},
      {"language":"rust","configuration":"packages/b/Cargo.toml","source_prefix":null,
       "declared_inputs":["missing.lock"]}
    ]}"#;
    let f = Fixture::new(manifest);
    let out = f.matrix(None, true);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar(
            "SELECT count(*) FROM sqlite_master WHERE name='evidence_receipt_v1'",
            &[]
        )
        .unwrap(),
        "0"
    );
}
