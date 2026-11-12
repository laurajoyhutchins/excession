//! Exact commit indexing must not read uncommitted checkout bytes.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(cwd: &Path, program: &str, args: &[&str]) -> Output {
    let out = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("launch test command");
    assert!(
        out.status.success(),
        "{} {:?} failed: {}",
        program,
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn git(cwd: &Path, args: &[&str]) -> String {
    String::from_utf8(run(cwd, "git", args).stdout)
        .expect("Git UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn index_of_pinned_git_bytes_is_identical_in_dirty_checkout() {
    let root = std::env::temp_dir().join(format!(
        "excession-dirty-git-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _cleaner = Fixture(root.clone());
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("pkg")).unwrap();
    fs::create_dir_all(root.join("out")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("pkg/__init__.py"), "").unwrap();
    fs::write(repo.join("pkg/a.py"), "from pkg import b\n").unwrap();
    fs::write(repo.join("pkg/b.py"), "VALUE = 1\n").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
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
    let expected = git(&repo, &["rev-parse", "HEAD"]);
    let bin = env!("CARGO_BIN_EXE_excession");
    let repo_path = repo.to_str().unwrap();
    let clean_db = root.join("out/clean.sqlite");
    let dirty_db = root.join("out/dirty.sqlite");
    let indexed = |path: &Path| {
        run(
            &repo,
            bin,
            &[
                "index",
                "--repo",
                repo_path,
                "--db",
                path.to_str().unwrap(),
                "--name",
                "test",
                "--rev",
                "HEAD",
                "--roots",
                "pkg",
            ],
        )
        .stdout
    };
    let clean = indexed(&clean_db);
    // Stage one file, modify a second without staging, and add an untracked file.
    fs::write(repo.join("pkg/b.py"), "VALUE = 99\n").unwrap();
    git(&repo, &["add", "pkg/b.py"]);
    fs::write(
        repo.join("pkg/a.py"),
        "raise RuntimeError('should not execute')\n",
    )
    .unwrap();
    fs::write(repo.join("untracked.py"), "UNKNOWN = True\n").unwrap();
    let status_before = git(&repo, &["status", "--porcelain", "-uall"]);
    assert!(!status_before.is_empty());
    let dirty = indexed(&dirty_db);
    assert_eq!(clean, dirty, "pinned Git-derived index metadata changed");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), expected);
    assert_eq!(
        git(&repo, &["status", "--porcelain", "-uall"]),
        status_before
    );
    let inspect = |path: &Path| {
        run(
            &repo,
            bin,
            &[
                "inspect",
                "dependencies",
                "--db",
                path.to_str().unwrap(),
                "--name",
                "test",
                "--rev",
                "HEAD",
            ],
        )
        .stdout
    };
    assert_eq!(inspect(&clean_db), inspect(&dirty_db));
}
