//! Real native compiler/typechecker executions over independently committed
//! mixed-language sources. Successful check != artifact build attestation.
use excession::json::{self, Value};
use excession::sqlite::Db;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}
fn success(program: &str, args: &[&str], dir: &Path) -> String {
    let result = run(program, args, dir);
    assert!(
        result.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}
fn available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn check(out: Output, success_expected: bool) -> Value {
    assert_eq!(
        out.status.success(),
        success_expected,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result = json::parse(&out.stdout).unwrap();
    assert_eq!(
        result.get("status").unwrap().str().unwrap(),
        if success_expected { "PASS" } else { "FAIL" }
    );
    assert!(result.get("report_id").unwrap().str().unwrap().len() == 64);
    assert_eq!(
        result.get("execution_authority").unwrap().str().unwrap(),
        "TRUSTED_LOCAL_NOT_ATTESTED"
    );
    assert_eq!(
        result
            .get("artifact_build_provenance")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert_eq!(result.get("rea").unwrap().str().unwrap(), "NOT_ESTABLISHED");
    result
}
struct Fixture {
    temp: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    commit: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-compiler-check-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = temp.join("clean source tree");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("web")).unwrap();
        fs::create_dir_all(repo.join("py")).unwrap();
        success("git", &["init", "-q"], &repo);
        for (name, data) in [
            (
                "Cargo.toml",
                "[package]\nname = \"excession-compiler-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            (
                "tsconfig.json",
                "{\"compilerOptions\":{\"strict\":true,\"target\":\"ES2022\"},\"include\":[\"web/**/*.ts\"]}\n",
            ),
            (
                "web/main.ts",
                "export function answer(): number { return 42; }\n",
            ),
            ("mypy.ini", "[mypy]\nstrict = True\n"),
            ("py/__init__.py", ""),
            ("py/mathlib.py", "def answer() -> int:\n    return 42\n"),
        ] {
            fs::write(repo.join(name), data).unwrap();
        }
        success("cargo", &["generate-lockfile", "--offline"], &repo);
        success("git", &["add", "."], &repo);
        success(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "initial source",
            ],
            &repo,
        );
        let commit = success("git", &["rev-parse", "HEAD"], &repo);
        let db = temp.join("proof.sqlite");
        Self {
            temp,
            repo,
            db,
            commit,
        }
    }
    fn invoke(
        &self,
        language: &str,
        config: &str,
        source_prefix: Option<&str>,
        tool: Option<&Path>,
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_excession"));
        command
            .args(["check", "run", "--repo"])
            .arg(&self.repo)
            .args([
                "--rev",
                &self.commit,
                "--language",
                language,
                "--config",
                config,
                "--db",
            ])
            .arg(&self.db)
            .args(["--trusted-local", "true"]);
        if let Some(scope) = source_prefix {
            command.args(["--source-prefix", scope]);
        }
        if let Some(path) = tool {
            command.arg("--tool").arg(path);
        }
        command.output().unwrap()
    }
    fn audit(&self, snapshot: &str, rev: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_excession"))
            .args(["check", "audit", "--repo"])
            .arg(&self.repo)
            .args(["--rev", rev, "--db"])
            .arg(&self.db)
            .args(["--snapshot", snapshot])
            .output()
            .unwrap()
    }
    fn changed(&mut self, path: &str, content: &str) {
        fs::write(self.repo.join(path), content).unwrap();
        success("git", &["add", path], &self.repo);
        success(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "changed source",
            ],
            &self.repo,
        );
        self.commit = success("git", &["rev-parse", "HEAD"], &self.repo);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}
fn assert_git_binding(report: &Value, f: &Fixture, configuration: &str) {
    assert_eq!(
        report.get("source_commit").unwrap().str().unwrap(),
        f.commit
    );
    let config = report.get("configuration").unwrap();
    assert_eq!(config.get("path").unwrap().str().unwrap(), configuration);
    assert_eq!(
        config.get("blob").unwrap().str().unwrap(),
        success(
            "git",
            &["rev-parse", &format!("HEAD:{configuration}")],
            &f.repo
        )
    );
}

#[test]
fn real_cargo_check_is_git_bound_auditable_and_not_a_build_receipt() {
    let f = Fixture::new();
    let report = check(f.invoke("rust", "Cargo.toml", None, None), true);
    assert_git_binding(&report, &f, "Cargo.toml");
    assert_eq!(
        report.get("profile").unwrap().str().unwrap(),
        "CARGO_CHECK_FROZEN_ALL_TARGETS"
    );
    let snapshot = report.get("snapshot_id").unwrap().str().unwrap();
    let replay = json::parse(&f.audit(snapshot, "HEAD").stdout).unwrap();
    assert_eq!(replay, report);
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_receipt_v1", &[])
            .unwrap(),
        "1"
    );
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_observation_v1", &[])
            .unwrap(),
        "1"
    );
    assert!(!f.repo.join("target").exists());
}

#[test]
fn compiler_failure_still_persists_as_fail_and_never_promotes_build_provenance() {
    let mut f = Fixture::new();
    f.changed(
        "src/lib.rs",
        "pub fn answer() -> u32 { \"not an integer\" }\n",
    );
    let report = check(f.invoke("rust", "Cargo.toml", None, None), false);
    assert_eq!(
        report
            .get("execution")
            .unwrap()
            .get("exit_code")
            .unwrap()
            .integer()
            .unwrap(),
        101
    );
    let snapshot = report.get("snapshot_id").unwrap().str().unwrap();
    let replay = f.audit(snapshot, "HEAD");
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(json::parse(&replay.stdout).unwrap(), report);
}

#[test]
fn invalid_trust_and_dirty_git_checkouts_are_refused_before_compiler_invocation() {
    let f = Fixture::new();
    let untrusted = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["check", "run", "--repo"])
        .arg(&f.repo)
        .args(["--language", "rust", "--config", "Cargo.toml", "--db"])
        .arg(&f.db)
        .args(["--trusted-local", "false"])
        .output()
        .unwrap();
    assert!(!untrusted.status.success());
    assert!(untrusted.stdout.is_empty());
    assert!(String::from_utf8_lossy(&untrusted.stderr).contains("trusted-local"));
    fs::write(f.repo.join("src/lib.rs"), "pub fn edited() {}\n").unwrap();
    let dirty = f.invoke("rust", "Cargo.toml", None, None);
    assert!(!dirty.status.success());
    assert!(dirty.stdout.is_empty());
    assert!(String::from_utf8_lossy(&dirty.stderr).contains("dirty checkout"));
}

#[test]
fn stale_revision_and_tampered_sqlite_observations_cannot_audit() {
    let mut f = Fixture::new();
    let report = check(f.invoke("rust", "Cargo.toml", None, None), true);
    let snapshot = report.get("snapshot_id").unwrap().str().unwrap().to_owned();
    let old = f.commit.clone();
    f.changed("src/lib.rs", "pub fn changed() -> u32 { 43 }\n");
    let wrong = f.audit(&snapshot, "HEAD");
    assert!(!wrong.status.success());
    assert!(f.audit(&snapshot, &old).status.success());
    let db = Db::open(&f.db).unwrap();
    db.exec("UPDATE evidence_observation_v1 SET document='{}'")
        .unwrap();
    assert!(!f.audit(&snapshot, &old).status.success());
}

#[test]
fn actual_typescript_project_typecheck_and_failure_are_both_observable() {
    let ts = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("node_modules/.bin/tsc");
    if !ts.is_file() {
        // The suite runs in a portable minimal Rust build too; dedicated CI
        // installs the project's pinned native TypeScript 5.8.3 launcher.
        return;
    }
    let ts = ts.canonicalize().unwrap();
    let mut f = Fixture::new();
    let report = check(
        f.invoke("typescript", "tsconfig.json", None, Some(&ts)),
        true,
    );
    assert_git_binding(&report, &f, "tsconfig.json");
    assert_eq!(
        report.get("profile").unwrap().str().unwrap(),
        "TSC_NO_EMIT_SINGLE_PROJECT"
    );
    assert!(
        f.audit(report.get("snapshot_id").unwrap().str().unwrap(), "HEAD")
            .status
            .success()
    );
    assert!(!f.repo.join("tsconfig.tsbuildinfo").exists());
    f.changed(
        "web/main.ts",
        "export function answer(): number { return 'wrong'; }\n",
    );
    let failed = check(
        f.invoke("typescript", "tsconfig.json", None, Some(&ts)),
        false,
    );
    assert!(
        failed
            .get("execution")
            .unwrap()
            .get("stdout_preview")
            .unwrap()
            .str()
            .unwrap()
            .contains("error")
    );
}

#[test]
fn installed_mypy_performs_a_configured_python_check_without_relabeling_as_build() {
    if !available("mypy") {
        return;
    }
    let mut f = Fixture::new();
    let passed = check(f.invoke("python", "mypy.ini", Some("py"), None), true);
    assert_git_binding(&passed, &f, "mypy.ini");
    assert_eq!(passed.get("source_scope").unwrap().str().unwrap(), "py");
    assert!(
        f.audit(passed.get("snapshot_id").unwrap().str().unwrap(), "HEAD")
            .status
            .success()
    );
    f.changed("py/mathlib.py", "def answer() -> int:\n    return 'bad'\n");
    check(f.invoke("python", "mypy.ini", Some("py"), None), false);
}
