//! Real pinned Git, native Python index, semantic compilation, and additive
//! refactor evidence join. Syntax receipts remain deliberately synthetic.
use excession::concepts;
use excession::git;
use excession::json::{self, Value, obj};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn call(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}

fn ok(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git_cmd(repo: &Path, args: &[&str]) -> Vec<u8> {
    ok(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap())
}

fn cli(args: &[&str]) -> Value {
    json::parse(&ok(call(env!("CARGO_BIN_EXE_excession"), args))).unwrap()
}

struct Fixture {
    tmp: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    report: PathBuf,
    commit: String,
    tree: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

impl Fixture {
    fn new() -> Self {
        let tmp = std::env::temp_dir().join(format!(
            "excession-concept-annotation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = tmp.join("repo");
        fs::create_dir_all(repo.join("pkg")).unwrap();
        git_cmd(&repo, &["init", "-q"]);
        git_cmd(&repo, &["config", "user.name", "Annotation fixture"]);
        git_cmd(&repo, &["config", "user.email", "fixture@example.invalid"]);
        fs::write(repo.join("pkg/__init__.py"), "").unwrap();
        fs::write(repo.join("pkg/a.py"), "from pkg import b\n").unwrap();
        fs::write(repo.join("pkg/b.py"), "from pkg import a\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-qm", "Pinned package cycle"]);
        let commit = git::rev(&repo, "HEAD").unwrap();
        let tree = git::rev(&repo, &format!("{commit}^{{tree}}")).unwrap();
        let db = tmp.join("test.sqlite");
        cli(&[
            "index",
            "--repo",
            repo.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--name",
            "example",
            "--rev",
            &commit,
            "--roots",
            "pkg",
        ]);
        Self {
            report: tmp.join("observations.json"),
            tmp,
            repo,
            db,
            commit,
            tree,
        }
    }

    fn observation(&self, from: &str, to: &str) -> Value {
        concepts::seal(
            obj(&[
                ("kind", Value::string("IMPORTS")),
                (
                    "subject",
                    obj(&[
                        ("path", Value::string(from)),
                        (
                            "blob",
                            Value::string(git::blob(&self.repo, &self.commit, from).unwrap()),
                        ),
                        ("line", Value::number(1)),
                        ("language", Value::string("python")),
                    ]),
                ),
                (
                    "target",
                    obj(&[
                        ("path", Value::string(to)),
                        (
                            "blob",
                            Value::string(git::blob(&self.repo, &self.commit, to).unwrap()),
                        ),
                    ]),
                ),
                (
                    "specifier",
                    Value::string(format!(
                        "pkg.{}",
                        to.trim_start_matches("pkg/").trim_end_matches(".py")
                    )),
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
            ]),
            "observation_id",
        )
        .unwrap()
    }

    fn populate(&self, reciprocal: bool) {
        let mut records = vec![self.observation("pkg/a.py", "pkg/b.py")];
        if reciprocal {
            records.push(self.observation("pkg/b.py", "pkg/a.py"));
        }
        records.sort_by_key(Value::canonical);
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
                ("record_count", Value::number(records.len())),
                ("gap_count", Value::number(0)),
                ("records", Value::array(records)),
                ("gaps", Value::array(vec![])),
            ]),
            "report_id",
        )
        .unwrap();
        fs::write(&self.report, report.canonical()).unwrap();
    }

    fn compile(&self) -> Value {
        cli(&[
            "concepts",
            "compile",
            "--repo",
            self.repo.to_str().unwrap(),
            "--report",
            self.report.to_str().unwrap(),
            "--db",
            self.db.to_str().unwrap(),
        ])
    }

    fn annotate(&self, snapshot: &str) -> Value {
        cli(&[
            "concepts",
            "annotate-refactor",
            "--db",
            self.db.to_str().unwrap(),
            "--name",
            "example",
            "--rev",
            &self.commit,
            "--snapshot",
            snapshot,
        ])
    }
}

#[test]
fn exact_source_overlap_adds_only_an_unverified_annotation() {
    let f = Fixture::new();
    f.populate(true);
    let baseline = cli(&[
        "refactor",
        "suggest",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "example",
        "--rev",
        &f.commit,
    ]);
    let manifest = f.compile();
    let snapshot = manifest.get("snapshot_id").unwrap().str().unwrap();
    let result = f.annotate(snapshot);
    assert_eq!(
        result.get("analysis_report_id").unwrap(),
        baseline.get("report_id").unwrap()
    );
    assert_eq!(
        result.get("coverage_status").unwrap().str().unwrap(),
        "PARTIAL"
    );
    let links = result.get("links").unwrap().array_ref().unwrap();
    assert!(
        !links.is_empty(),
        "expected a source-bound reciprocal import cycle"
    );
    for link in links {
        assert_eq!(
            link.get("assessment").unwrap().str().unwrap(),
            "ANNOTATION_ONLY"
        );
        assert_eq!(
            link.get("role").unwrap().str().unwrap(),
            "CONCEPT_ANNOTATION"
        );
        assert!(
            !link
                .get("concept_claim_ids")
                .unwrap()
                .array_ref()
                .unwrap()
                .is_empty()
        );
    }
    let unchanged = cli(&[
        "refactor",
        "suggest",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "example",
        "--rev",
        &f.commit,
    ]);
    assert_eq!(baseline, unchanged);
}

#[test]
fn unreciprocated_claims_produce_no_refactor_annotation() {
    let f = Fixture::new();
    f.populate(false);
    let manifest = f.compile();
    let snapshot = manifest.get("snapshot_id").unwrap().str().unwrap();
    let result = f.annotate(snapshot);
    assert!(result.get("links").unwrap().array_ref().unwrap().is_empty());
}
