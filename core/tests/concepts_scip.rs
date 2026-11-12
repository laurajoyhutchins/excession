//! Git-backed SCIP semantic bridge acceptance. The SCIP JSON is a synthetic
//! official-decoder-shaped fixture; producer execution is not attested.
use excession::json::{self, Value, obj};
use excession::{concepts_compile, concepts_scip, scip_refactor};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const RUST_SYMBOL: &str = "scip rust cargo sample 0.1.0 alpha/alpha().";
const PYTHON_SYMBOL: &str = "scip python pypi sample 0.1.0 beta/beta().";
const EXTERNAL_SYMBOL: &str = "scip python pypi unknown 0.1.0 external/f().";

fn output(program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .output()
        .expect("spawn program")
}

fn ok(result: Output) -> Vec<u8> {
    assert!(
        result.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("start Git");
    String::from_utf8(ok(output)).unwrap().trim().to_owned()
}

fn occurrence(symbol: &str, role: i64, line: i64) -> Value {
    obj(&[
        ("symbol", Value::string(symbol)),
        ("symbol_roles", Value::number(role)),
        (
            "range",
            Value::array([Value::number(line), Value::number(0), Value::number(2)]),
        ),
    ])
}

fn document(path: &str, lang: &str, occurrences: Vec<Value>) -> Value {
    obj(&[
        ("relative_path", Value::string(path)),
        ("language", Value::string(lang)),
        ("position_encoding", Value::number(1)),
        ("occurrences", Value::array(occurrences)),
    ])
}

struct Fixture {
    dir: PathBuf,
    repo: PathBuf,
    index: PathBuf,
    db: PathBuf,
    commit: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "excession-scip-concepts-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = dir.join("source");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "SCIP acceptance"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        fs::write(repo.join("alpha.rs"), "fn alpha() {}\n").unwrap();
        fs::write(repo.join("beta.py"), "def beta():\n    return 1\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "SCIP pinned source"]);
        let commit = git(&repo, &["rev-parse", "HEAD"]);
        let index = dir.join("scip.json");
        let db = dir.join("semantic.sqlite");
        let fixture = Self {
            dir,
            repo,
            index,
            db,
            commit,
        };
        fixture.write_index(true, false);
        fixture
    }

    fn write_index(&self, reciprocal: bool, ambiguous: bool) {
        let rust = vec![
            occurrence(RUST_SYMBOL, 1, 0),
            occurrence(PYTHON_SYMBOL, 0, 0),
            occurrence(EXTERNAL_SYMBOL, 0, 0),
        ];
        let mut python = vec![occurrence(PYTHON_SYMBOL, 1, 0)];
        if reciprocal {
            python.push(occurrence(RUST_SYMBOL, 0, 0));
        }
        // Keep this synthetic input intentionally minimal but strictly shaped
        // like the official SCIP CLI JSON.
        let mut documents = vec![
            document("alpha.rs", "rust", rust),
            document("beta.py", "python", python),
        ];
        if ambiguous {
            fs::write(self.repo.join("other.py"), "def beta():\n    return 2\n").unwrap();
            // This file isn't tracked in the pinned tree, which is rejected
            // before an ambiguous source could be mistaken for evidence.
            documents.push(document(
                "other.py",
                "python",
                vec![occurrence(PYTHON_SYMBOL, 1, 0)],
            ));
        }
        let value = obj(&[
            (
                "metadata",
                obj(&[(
                    "tool_info",
                    obj(&[
                        ("name", Value::string("synthetic-official-scip-json")),
                        ("version", Value::string("test-v1")),
                    ]),
                )]),
            ),
            ("documents", Value::array(documents)),
        ]);
        fs::write(&self.index, value.canonical()).unwrap();
    }

    fn cli(&self, verb: &str, extra: &[&str]) -> Value {
        let mut args = vec![
            "concepts",
            verb,
            "--repo",
            self.repo.to_str().unwrap(),
            "--rev",
            &self.commit,
            "--index",
            self.index.to_str().unwrap(),
        ];
        args.extend(extra);
        json::parse(&ok(output(env!("CARGO_BIN_EXE_excession"), &args))).unwrap()
    }
}

#[test]
fn real_git_bound_scip_symbols_compile_without_inventing_imports_or_proof() {
    let f = Fixture::new();
    let original = scip_refactor::suggest(&f.repo, &f.commit, &f.index).unwrap();
    assert_eq!(
        original.get("status").unwrap().str().unwrap(),
        "UNVERIFIED_SCIP_RELATIONS"
    );
    assert_eq!(
        original
            .get("candidates")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        3
    );

    let native = scip_refactor::references(&f.repo, &f.commit, &f.index).unwrap();
    let refs = native.get("references").unwrap().array_ref().unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(
        native
            .get("coverage")
            .unwrap()
            .get("unresolved_references")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );

    let normalized = f.cli("scip-normalize", &[]);
    assert_eq!(
        normalized,
        concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).unwrap()
    );
    let observations = normalized.get("observations").unwrap().array_ref().unwrap();
    assert_eq!(observations.len(), 2);
    assert!(
        observations
            .iter()
            .all(|item| item.get("kind").unwrap().str().unwrap() == "SCIP_REFERENCE")
    );
    assert!(
        observations
            .iter()
            .all(|item| item.get("native_relation").unwrap().str().unwrap()
                == "SCIP_SYMBOL_REFERENCE")
    );
    assert!(
        observations
            .iter()
            .all(|item| item.get("status").unwrap().str().unwrap() == "OBSERVED")
    );
    assert!(normalized.get("gaps").unwrap().array_ref().unwrap().len() >= 2);

    let preview = f.cli("scip-preview", &[]);
    assert_eq!(preview, concepts_compile::compile(&normalized).unwrap());
    assert_eq!(
        preview.get("concepts").unwrap().array_ref().unwrap().len(),
        1
    );
    for claim in preview.get("claims").unwrap().array_ref().unwrap() {
        assert_eq!(
            claim.get("behavioral_verification").unwrap().str().unwrap(),
            "UNTESTED"
        );
        assert_eq!(claim.get("assessment").unwrap().str().unwrap(), "PROPOSED");
        for support in claim.get("support").unwrap().array_ref().unwrap() {
            assert_eq!(
                support.get("observation_kind").unwrap().str().unwrap(),
                "SCIP_REFERENCE"
            );
            assert!(support.get("observation_id").is_ok());
        }
    }
    let db = f.db.to_str().unwrap();
    let first = f.cli("scip-compile", &["--db", db]);
    assert_eq!(first, f.cli("scip-compile", &["--db", db]));
    let id = first.get("snapshot_id").unwrap().str().unwrap();
    let audit = f.cli("scip-audit", &["--db", db, "--snapshot", id]);
    assert_eq!(
        audit.get("status").unwrap().str().unwrap(),
        "SOURCE_BOUND_SCIP_INDEX_REPLAYED"
    );
    assert!(
        !audit
            .get("indexer_independently_executed")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert_eq!(
        original,
        scip_refactor::suggest(&f.repo, &f.commit, &f.index).unwrap()
    );
}

#[test]
fn one_way_scip_symbol_reference_is_not_an_ontology_merge() {
    let f = Fixture::new();
    f.write_index(false, false);
    let batch = f.cli("scip-normalize", &[]);
    assert_eq!(
        batch
            .get("observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
    let model = f.cli("scip-preview", &[]);
    assert!(
        model
            .get("concepts")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert!(model.get("claims").unwrap().array_ref().unwrap().is_empty());
    assert_eq!(
        model
            .get("coverage")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "PARTIAL"
    );
}

#[test]
fn stale_or_untracked_git_sources_fail_closed() {
    let f = Fixture::new();
    f.write_index(true, true);
    assert!(concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).is_err());
    let result = output(
        env!("CARGO_BIN_EXE_excession"),
        &[
            "concepts",
            "scip-compile",
            "--repo",
            f.repo.to_str().unwrap(),
            "--rev",
            &f.commit,
            "--index",
            f.index.to_str().unwrap(),
            "--db",
            f.db.to_str().unwrap(),
        ],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());

    f.write_index(true, false);
    let mut raw = json::parse(&fs::read(&f.index).unwrap()).unwrap();
    let docs = raw.entries_mut().unwrap().get_mut("documents").unwrap();
    let first = match docs {
        Value::Array(items) => &mut items[0],
        _ => panic!("expected SCIP documents"),
    };
    let occurrences = first.entries_mut().unwrap().get_mut("occurrences").unwrap();
    let occurrence = match occurrences {
        Value::Array(items) => &mut items[0],
        _ => panic!("expected SCIP occurrences"),
    };
    occurrence.entries_mut().unwrap().insert(
        "range".into(),
        Value::array([Value::number(0), Value::number(500), Value::number(502)]),
    );
    fs::write(&f.index, raw.canonical()).unwrap();
    assert!(concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).is_err());
}

#[test]
fn nested_source_prefix_requires_exact_scip_project_root() {
    let f = Fixture::new();
    let refused = output(
        env!("CARGO_BIN_EXE_excession"),
        &[
            "concepts",
            "scip-normalize",
            "--repo",
            f.repo.to_str().unwrap(),
            "--rev",
            &f.commit,
            "--index",
            f.index.to_str().unwrap(),
            "--source-prefix",
            "subproject",
        ],
    );
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
}

#[test]
fn duplicate_symbol_definitions_in_one_file_are_not_a_unique_reference_target() {
    let f = Fixture::new();
    let mut index = json::parse(&fs::read(&f.index).unwrap()).unwrap();
    let documents = index.entries_mut().unwrap().get_mut("documents").unwrap();
    let python = match documents {
        Value::Array(items) => &mut items[1],
        _ => panic!("expected SCIP documents"),
    };
    let occurrences = python
        .entries_mut()
        .unwrap()
        .get_mut("occurrences")
        .unwrap();
    match occurrences {
        Value::Array(items) => items.push(occurrence(PYTHON_SYMBOL, 1, 0)),
        _ => panic!("expected SCIP occurrences"),
    }
    fs::write(&f.index, index.canonical()).unwrap();

    let native = scip_refactor::references(&f.repo, &f.commit, &f.index).unwrap();
    assert_eq!(
        native.get("references").unwrap().array_ref().unwrap().len(),
        1
    );
    assert_eq!(
        native
            .get("coverage")
            .unwrap()
            .get("unresolved_references")
            .unwrap()
            .integer()
            .unwrap(),
        2
    );
    let model = f.cli("scip-preview", &[]);
    assert!(
        model
            .get("concepts")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert!(model.get("claims").unwrap().array_ref().unwrap().is_empty());
}
