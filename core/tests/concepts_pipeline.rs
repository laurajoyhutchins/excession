//! End-to-end CLI acceptance over a real pinned Git revision.
//! Synthetic syntax receipts are only test inputs, not a native parser oracle.
use excession::concepts;
use excession::git;
use excession::json::{self, Value, obj};
use excession::sqlite::{Db, Param};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn cmd(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git_cmd(repo: &Path, args: &[&str]) -> Vec<u8> {
    let mut git = Command::new("git");
    success(git.arg("-C").arg(repo).args(args).output().unwrap())
}

fn cli(args: &[&str]) -> Value {
    json::parse(&success(cmd(env!("CARGO_BIN_EXE_excession"), args))).unwrap()
}

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    report: PathBuf,
    db: PathBuf,
    commit: String,
    tree: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-concepts-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git_cmd(&repo, &["init", "-q"]);
        git_cmd(&repo, &["config", "user.name", "Concept acceptance"]);
        git_cmd(&repo, &["config", "user.email", "concepts@example.invalid"]);
        fs::write(repo.join("a.py"), "from . import b\n").unwrap();
        fs::write(repo.join("b.py"), "from . import a\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-qm", "Pinned concept source"]);
        let commit = git::rev(&repo, "HEAD").unwrap();
        let tree = git::rev(&repo, &format!("{commit}^{{tree}}")).unwrap();
        Self {
            report: root.join("observations.json"),
            db: root.join("snapshot.db"),
            root,
            repo,
            commit,
            tree,
        }
    }

    fn observation(&self, from: &str, to: &str) -> Value {
        let target = git::blob(&self.repo, &self.commit, to).unwrap();
        let source = git::blob(&self.repo, &self.commit, from).unwrap();
        concepts::seal(
            obj(&[
                ("kind", Value::string("IMPORTS")),
                (
                    "subject",
                    obj(&[
                        ("path", Value::string(from)),
                        ("blob", Value::string(source)),
                        ("line", Value::number(1)),
                        ("language", Value::string("python")),
                    ]),
                ),
                (
                    "specifier",
                    Value::string(format!(".{}", to.trim_end_matches(".py"))),
                ),
                (
                    "assessment",
                    obj(&[
                        ("status", Value::string("CANDIDATE")),
                        ("method", Value::string("SYNTAX_RESOLUTION")),
                        ("rule", Value::string("RELATIVE_PATH_CANDIDATE")),
                    ]),
                ),
                ("condition_evaluation", Value::string("NOT_EVALUATED")),
                (
                    "target",
                    obj(&[("path", Value::string(to)), ("blob", Value::string(target))]),
                ),
            ]),
            "observation_id",
        )
        .unwrap()
    }

    fn report(&self, reciprocal: bool) {
        let mut observations = vec![self.observation("a.py", "b.py")];
        if reciprocal {
            observations.push(self.observation("b.py", "a.py"));
        }
        observations.sort_by_key(Value::canonical);
        let report = concepts::seal(
            obj(&[
                (
                    "schema",
                    Value::string("excession.semantic-observations.v1"),
                ),
                ("source_commit", Value::string(&self.commit)),
                ("source_tree", Value::string(&self.tree)),
                ("syntax_report_id", Value::string("a".repeat(64))),
                ("resolution_report_id", Value::string("b".repeat(64))),
                ("completeness", Value::string("PARTIAL_CANDIDATES_ONLY")),
                ("record_count", Value::number(observations.len())),
                ("gap_count", Value::number(0)),
                ("records", Value::array(observations)),
                ("gaps", Value::array(vec![])),
            ]),
            "report_id",
        )
        .unwrap();
        fs::write(&self.report, report.canonical()).unwrap();
    }

    fn params(&self, command: &str) -> Vec<String> {
        vec![
            "concepts".into(),
            command.into(),
            "--repo".into(),
            self.repo.to_str().unwrap().into(),
            "--report".into(),
            self.report.to_str().unwrap().into(),
        ]
    }

    fn invoke(&self, command: &str, extra: &[&str]) -> Value {
        let mut args = self.params(command);
        args.extend(extra.iter().map(|x| (*x).into()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli(&refs)
    }
}

#[test]
fn reciprocal_candidates_compile_as_partial_not_verified() {
    let fixture = Fixture::new();
    fixture.report(true);
    let normalized = fixture.invoke("normalize", &[]);
    assert_eq!(
        normalized.get("completeness").unwrap().str().unwrap(),
        "PARTIAL_CANDIDATES_ONLY"
    );
    let preview = fixture.invoke("preview", &[]);
    assert_eq!(
        preview.get("concepts").unwrap().array_ref().unwrap().len(),
        1
    );
    let claims = preview.get("claims").unwrap().array_ref().unwrap();
    assert_eq!(claims.len(), 3);
    for claim in claims {
        assert_eq!(claim.get("assessment").unwrap().str().unwrap(), "PROPOSED");
        assert_eq!(
            claim.get("behavioral_verification").unwrap().str().unwrap(),
            "UNTESTED"
        );
    }
    let db = fixture.db.to_str().unwrap();
    let manifest = fixture.invoke("compile", &["--db", db]);
    assert_eq!(manifest, fixture.invoke("compile", &["--db", db]));
    let snapshot = manifest.get("snapshot_id").unwrap().str().unwrap();
    let audited = fixture.invoke("audit", &["--db", db, "--snapshot", snapshot]);
    assert_eq!(
        audited.get("status").unwrap().str().unwrap(),
        "SOURCE_BINDINGS_RECHECKED"
    );
    assert!(!audited.get("producer_reexecuted").unwrap().bool().unwrap());
    let store = Db::open(&fixture.db).unwrap();
    store
        .execute(
            "INSERT INTO concept_subject VALUES (?,?,?)",
            &[
                Param::Text(snapshot),
                Param::Text(&"f".repeat(64)),
                Param::Text("{}"),
            ],
        )
        .unwrap();
    let mut args = fixture.params("audit");
    args.extend([
        "--db".into(),
        db.into(),
        "--snapshot".into(),
        snapshot.into(),
    ]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let refused = cmd(env!("CARGO_BIN_EXE_excession"), &refs);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
}

#[test]
fn one_way_import_does_not_create_architectural_concept() {
    let fixture = Fixture::new();
    fixture.report(false);
    let preview = fixture.invoke("preview", &[]);
    assert!(
        preview
            .get("concepts")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert!(
        preview
            .get("claims")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        preview
            .get("coverage")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "PARTIAL"
    );
}
