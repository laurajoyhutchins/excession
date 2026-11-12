//! End-to-end explanation replays its source-bound impact claim rather than trusting JSON.
use excession::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn real_git_explanation_replays_and_refuses_tampered_claim() {
    let root: PathBuf = std::env::temp_dir().join(format!(
        "excession-explain-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("pkg")).unwrap();
    let cmd = |name: &str, args: &[&str]| -> std::process::Output {
        Command::new(name)
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap()
    };
    let ok = |name: &str, args: &[&str]| -> String {
        let out = cmd(name, args);
        assert!(
            out.status.success(),
            "{} {:?}: {}",
            name,
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    ok("git", &["init", "-q"]);
    fs::write(repo.join("pkg/__init__.py"), "").unwrap();
    fs::write(repo.join("pkg/a.py"), "import pkg.b\n").unwrap();
    fs::write(repo.join("pkg/b.py"), "VALUE = 1\n").unwrap();
    ok("git", &["add", "."]);
    ok(
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "before",
        ],
    );
    let before = ok("git", &["rev-parse", "HEAD"]);
    fs::write(repo.join("pkg/b.py"), "VALUE = 2\n").unwrap();
    ok("git", &["add", "."]);
    ok(
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "after",
        ],
    );
    let bin = env!("CARGO_BIN_EXE_excession");
    let db = root.join("cache.sqlite");
    let path = root.join("impact.json");
    let impact = ok(
        bin,
        &[
            "impact",
            ".",
            "--base",
            &before,
            "--db",
            db.to_str().unwrap(),
            "--json",
            "--output",
            path.to_str().unwrap(),
        ],
    );
    let mut report = json::parse(impact.as_bytes()).unwrap();
    let effects = report.get("effects").unwrap().array_ref().unwrap();
    assert!(!effects.is_empty());
    let id = effects[0]
        .get("claim_id")
        .unwrap()
        .str()
        .unwrap()
        .to_owned();
    assert_eq!(fs::read_to_string(&path).unwrap().trim(), report.canonical());
    let explain = ok(
        bin,
        &[
            "explain",
            ".",
            "--report",
            path.to_str().unwrap(),
            "--id",
            &id,
            "--db",
            db.to_str().unwrap(),
            "--json",
        ],
    );
    let explanation = json::parse(explain.as_bytes()).unwrap();
    assert_eq!(
        explanation.get("schema").unwrap().str().unwrap(),
        "excession.explanation.v1"
    );
    assert_eq!(explanation.get("claim_id").unwrap().str().unwrap(), id);
    assert!(
        !explanation
            .get("steps")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    let audit = ok(
        bin,
        &[
            "explain",
            ".",
            "--report",
            path.to_str().unwrap(),
            "--id",
            &id,
            "--db",
            db.to_str().unwrap(),
            "--audit",
        ],
    );
    let audited = json::parse(audit.as_bytes()).unwrap();
    assert_eq!(
        audited.get("semantic_correctness").unwrap().str().unwrap(),
        "NOT_EVALUATED"
    );
    let mut changed = report.get("effects").unwrap().array_ref().unwrap().to_vec();
    let effect = changed.get_mut(0).unwrap();
    effect.entries_mut().unwrap().insert(
        "assessment".into(),
        json::Value::string("OBSERVED_RELATIONSHIP_PATH"),
    );
    report
        .entries_mut()
        .unwrap()
        .insert("effects".into(), json::Value::array(changed));
    fs::write(&path, report.canonical()).unwrap();
    let rejected = cmd(
        bin,
        &[
            "explain",
            ".",
            "--report",
            path.to_str().unwrap(),
            "--id",
            &id,
            "--db",
            db.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    fs::remove_dir_all(root).unwrap();
}
