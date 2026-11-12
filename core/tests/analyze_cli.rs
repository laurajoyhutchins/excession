//! The ordinary analyze path should consume a pinned Git revision and emit
//! source-bound *candidates*, without claiming complete dependency closure.
use excession::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn cmd(dir: &std::path::Path, exe: &str, args: &[&str]) -> Vec<u8> {
    let out = Command::new(exe)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{} {:?}: {}",
        exe,
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

#[test]
fn analyze_one_command_reports_git_bound_candidates_and_honest_limits() {
    let temp: PathBuf = std::env::temp_dir().join(format!(
        "excession-analyze-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = temp.join("repo");
    let pkg = repo.join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    cmd(&repo, "git", &["init", "-q"]);
    fs::write(pkg.join("__init__.py"), "").unwrap();
    fs::write(pkg.join("first.py"), "import pkg.second\n").unwrap();
    fs::write(pkg.join("second.py"), "VALUE = 1\n").unwrap();
    fs::write(repo.join("entrypoint.py"), "import pkg.second\n").unwrap();
    cmd(&repo, "git", &["add", "."]);
    cmd(
        &repo,
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "baseline",
        ],
    );
    let rev = String::from_utf8(cmd(&repo, "git", &["rev-parse", "HEAD"])).unwrap();
    let out = temp.join("cache");
    let bin = env!("CARGO_BIN_EXE_excession");
    let result = Command::new(bin)
        .current_dir(&repo)
        .arg("analyze")
        .arg(".")
        .arg("--db")
        .arg(out.join("index.sqlite"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json = json::parse(&result.stdout).unwrap();
    assert_eq!(
        json.get("schema").unwrap().str().unwrap(),
        "excession.analysis.v1"
    );
    assert_eq!(
        json.get("source_commit").unwrap().str().unwrap(),
        rev.trim()
    );
    assert_eq!(json.get("status").unwrap().str().unwrap(), "PARTIAL");
    let rels = json.get("relationships").unwrap().array_ref().unwrap();
    assert!(
        !rels.is_empty(),
        "expected the real import to yield a candidate"
    );
    assert!(
        rels.iter().any(|edge| {
            edge.get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                == "entrypoint.py"
        }),
        "root-level source must participate in default analysis"
    );
    assert!(rels.iter().all(|edge| {
        edge.get("evidence")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .all(|e| e.get("assessment").unwrap().str().unwrap() == "CANDIDATE")
    }));
    assert_eq!(
        json.get("coverage")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "PARTIAL"
    );
    assert!(!repo.join("excession.sqlite").exists());

    // Out-of-checkout policy must reject lexical traversal before any write.
    let traversal = out.join("..").join("repo").join("forbidden.sqlite");
    let refused = Command::new(bin)
        .current_dir(&repo)
        .args(["analyze", ".", "--db"])
        .arg(&traversal)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!repo.join("forbidden.sqlite").exists());

    // A symlinked parent can also route a nominally external path into Git.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let alias = temp.join("cache-alias");
        symlink(&repo, &alias).unwrap();
        let linked_path = alias.join("forbidden.sqlite");
        let refused = Command::new(bin)
            .current_dir(&repo)
            .args(["analyze", ".", "--db"])
            .arg(linked_path)
            .arg("--json")
            .output()
            .unwrap();
        assert!(!refused.status.success());
        assert!(!repo.join("forbidden.sqlite").exists());

        // A symlinked file target is unsafe even if its parent is external.
        let sentinel = repo.join("sentinel.sqlite");
        fs::write(&sentinel, b"leave this checkout file untouched").unwrap();
        let file_alias = temp.join("cache-file-alias.sqlite");
        symlink(&sentinel, &file_alias).unwrap();
        let refused = Command::new(bin)
            .current_dir(&repo)
            .args(["analyze", ".", "--db"])
            .arg(file_alias)
            .arg("--json")
            .output()
            .unwrap();
        assert!(!refused.status.success());
        assert_eq!(
            fs::read(sentinel).unwrap(),
            b"leave this checkout file untouched"
        );
    }

    let help = cmd(&repo, bin, &["analyze", "clang", "--help"]);
    assert!(String::from_utf8_lossy(&help).contains("compile-db"));
    fs::remove_dir_all(temp).unwrap();
}
