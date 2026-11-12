//! Reanalysis of an immutable Git commit must not preserve stale producer rows.
use excession::sqlite::Db;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "git failed: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn reanalysis_replaces_stale_derived_rows_for_the_same_commit() {
    let work = std::env::temp_dir().join(format!(
        "excession-cache-reanalysis-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _cleanup = Fixture(work.clone());
    let repo = work.join("repo");
    fs::create_dir_all(repo.join("pkg")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("pkg/__init__.py"), "").unwrap();
    fs::write(repo.join("pkg/a.py"), "import pkg.b\n").unwrap();
    fs::write(repo.join("pkg/b.py"), "VALUE = 1\n").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
          "commit", "-q", "-m", "baseline"],
    );
    let db_path = work.join("index.sqlite");
    let db = Db::open(&db_path).unwrap();
    let analyze = || {
        excession::index::index_revision_with_roots(
            &db, &repo, "fixture", "HEAD", "test", &["pkg"],
        ).unwrap();
    };
    analyze();
    let original = db.scalar("SELECT COUNT(*) FROM dependency", &[]).unwrap();
    assert_ne!(original, "0");

    // Simulate the old implementation retaining a parser's obsolete hazard.
    db.execute(
        "INSERT INTO hazard(repo_id,commit_oid,module_name,line,reason)
         SELECT repo_id,commit_oid,name,1,'OBSOLETE_ANALYZER_ROW'
         FROM module WHERE name='pkg.a'",
        &[],
    ).unwrap();
    assert_eq!(
        db.scalar("SELECT COUNT(*) FROM hazard WHERE reason='OBSOLETE_ANALYZER_ROW'", &[])
            .unwrap(),
        "1"
    );
    analyze();
    assert_eq!(
        db.scalar("SELECT COUNT(*) FROM hazard WHERE reason='OBSOLETE_ANALYZER_ROW'", &[])
            .unwrap(),
        "0"
    );
    assert_eq!(db.scalar("SELECT COUNT(*) FROM dependency", &[]).unwrap(), original);
    assert_eq!(
        db.query(
            "SELECT source_name,target_name FROM dependency ORDER BY source_name,target_name",
            &[],
        ).unwrap().len(),
        1
    );
}
