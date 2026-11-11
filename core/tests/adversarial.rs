//! End-to-end rejection controls against the real two-branch h11 Git bundle.
use relgit::integration::{Observations, bounded_carry, compare_reference};
use relgit::json::{self, Value};
use relgit::sqlite::{Db, Param};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "relgit-rust-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("unique fixture directory");
    for (branch, directory) in [
        ("experimental-base", "h11-base"),
        ("agent-A-headers", "agent-A-headers"),
        ("agent-B-reader", "agent-B-reader"),
        ("integration", "integration"),
    ] {
        let status = Command::new("git")
            .args(["clone", "-q", "--branch", branch])
            .arg(project().join("fixtures/h11.bundle"))
            .arg(root.join(directory))
            .status()
            .expect("git clone launch");
        assert!(status.success(), "cloning pinned fixture {branch}");
    }
    Fixture { root }
}
fn observations(f: &Fixture) -> Observations {
    let mut a =
        json::parse(&fs::read(project().join("fixtures/agent-A-headers.json")).unwrap()).unwrap();
    let mut b =
        json::parse(&fs::read(project().join("fixtures/agent-B-reader.json")).unwrap()).unwrap();
    set(
        &mut a,
        "module_origin",
        Value::string(
            f.root
                .join("agent-A-headers/h11/__init__.py")
                .to_string_lossy(),
        ),
    );
    set(
        &mut b,
        "module_origin",
        Value::string(
            f.root
                .join("agent-B-reader/h11/__init__.py")
                .to_string_lossy(),
        ),
    );
    Observations {
        headers: a,
        reader: b,
    }
}
fn set(obj: &mut Value, key: &str, val: Value) {
    obj_as_map(obj).insert(key.to_owned(), val);
}
fn obj_as_map(v: &mut Value) -> &mut std::collections::BTreeMap<String, Value> {
    if let Value::Object(o) = v {
        o
    } else {
        panic!("not object")
    }
}
fn boundary_mut(obs: &mut Value) -> &mut Vec<Value> {
    let x = obj_as_map(obs).get_mut("cross_boundary").unwrap();
    if let Value::Array(x) = x {
        x
    } else {
        panic!("not array")
    }
}
fn expect_refusal(f: &Fixture, o: &Observations, fragment: &str) {
    let err = bounded_carry(&f.root, o, None).expect_err("must refuse");
    assert!(err.contains(fragment), "wanted {fragment:?}, got {err:?}");
}
#[test]
fn real_history_and_cross_boundary_refusals() {
    let f = fixture();
    let pristine = observations(&f);
    let reference =
        json::parse(&fs::read(project().join("fixtures/reference_report.json")).unwrap()).unwrap();
    let baseline = bounded_carry(&f.root, &pristine, None).expect("actual control");
    compare_reference(&baseline, &reference).expect("exact differential");
    assert_eq!(
        baseline.get("carried_tests").unwrap().integer().unwrap(),
        16
    );
    let mut wrong = pristine.clone();
    set(&mut wrong.headers, "failed", Value::number(1));
    expect_refusal(&f, &wrong, "failed, skipped");
    let mut wrong = pristine.clone();
    set(&mut wrong.reader, "passed", Value::number(14));
    expect_refusal(&f, &wrong, "test-case count");
    let mut wrong = pristine.clone();
    set(&mut wrong.headers, "python", Value::string("0.0.0"));
    expect_refusal(&f, &wrong, "environment mismatch");
    let mut wrong = pristine.clone();
    set(
        &mut wrong.reader,
        "module_origin",
        Value::string(
            f.root
                .join("agent-A-headers/h11/__init__.py")
                .to_string_lossy(),
        ),
    );
    expect_refusal(&f, &wrong, "wrong checkout");
    let mut wrong = pristine.clone();
    boundary_mut(&mut wrong.reader).clear();
    expect_refusal(&f, &wrong, "missing cross-boundary");
    let mut wrong = pristine.clone();
    let boundary = boundary_mut(&mut wrong.headers);
    boundary.push(relgit::json::obj(&[
        ("kind", Value::string("reader-method")),
        ("name", Value::string("ChunkedReader.__call__")),
    ]));
    expect_refusal(&f, &wrong, "header tests invoked");
    let mut wrong = pristine.clone();
    let boundary = boundary_mut(&mut wrong.reader);
    let header = boundary
        .iter_mut()
        .find(|v| {
            v.get("kind")
                .is_ok_and(|k| k.str() == Ok("header-interface"))
        })
        .unwrap();
    set(header, "headers", Value::string("non-empty"));
    expect_refusal(&f, &wrong, "unverified inputs");
    // Dirty input check is independent of internal JSON correctness.
    fs::write(f.root.join("integration/UNTRACKED"), "unexpected file").unwrap();
    expect_refusal(&f, &pristine, "dirty checkout");
}
#[test]
fn exact_git_tree_sql_and_reference_guard() {
    let f = fixture();
    let obs = observations(&f);
    let dbpath = f.root.join("catalog.sqlite");
    let report = bounded_carry(&f.root, &obs, Some(&dbpath)).unwrap();
    let reference =
        json::parse(&fs::read(project().join("fixtures/reference_report.json")).unwrap()).unwrap();
    compare_reference(&report, &reference).unwrap();
    let db = Db::open(&dbpath).unwrap();
    db.foreign_keys_valid().unwrap();
    for (name, dir) in [
        ("base", "h11-base"),
        ("A", "agent-A-headers"),
        ("B", "agent-B-reader"),
        ("merged", "integration"),
    ] {
        let repo_id: i64 = db
            .scalar(
                "SELECT id FROM repository WHERE name=?",
                &[Param::Text(name)],
            )
            .unwrap()
            .parse()
            .unwrap();
        let commit = relgit::git::rev(&f.root.join(dir), "HEAD").unwrap();
        let tree = relgit::index::reconstruct_tree(&db, repo_id, &commit).unwrap();
        assert_eq!(
            tree,
            relgit::git::rev(&f.root.join(dir), "HEAD^{tree}").unwrap()
        );
    }
    assert!(db.execute("INSERT INTO premise(integration_key,ordinal,statement,satisfied) VALUES('not-real',99,'forged',1)",&[]).is_err(),"foreign key must reject bogus integration");
    // A legal-looking but false Git file mode must invalidate tree reconstruction.
    let merged_id: i64 = db
        .scalar("SELECT id FROM repository WHERE name='merged'", &[])
        .unwrap()
        .parse()
        .unwrap();
    let merged_commit = relgit::git::rev(&f.root.join("integration"), "HEAD").unwrap();
    db.execute(
        "UPDATE tree_entry SET mode='100755' WHERE repo_id=? AND commit_oid=? AND path='h11/_headers.py'",
        &[Param::Integer(merged_id), Param::Text(&merged_commit)],
    ).unwrap();
    assert!(
        relgit::index::reconstruct_tree(&db, merged_id, &merged_commit).is_err(),
        "tampered file mode must change Git root hash"
    );
    let mut falsified = reference.clone();
    set(
        &mut falsified,
        "guard_sha256",
        Value::string("0".repeat(64)),
    );
    assert!(compare_reference(&report, &falsified).is_err());
}
#[test]
fn changed_pinned_source_is_rejected() {
    let f = fixture();
    let obs = observations(&f);
    // The source is not allowed to change, even if the tested observation was previously successful.
    let source = f.root.join("agent-A-headers/h11/_headers.py");
    let original = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("{original}\n# undetected local edit\n")).unwrap();
    expect_refusal(&f, &obs, "dirty checkout");
}
