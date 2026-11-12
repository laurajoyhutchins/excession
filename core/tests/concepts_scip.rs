//! Git-backed SCIP semantic bridge acceptance. The SCIP JSON is a synthetic
//! official-decoder-shaped fixture; producer execution is not attested.
use excession::json::{self, Value, obj};
use excession::{concepts_author, concepts_compile, concepts_scip, concepts_value, scip_refactor};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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

#[test]
fn native_scip_value_is_a_source_bound_rust_comparison_not_a_certification() {
    let f = Fixture::new();
    let result = f.cli("scip-value", &[]);
    assert_eq!(
        result,
        concepts_value::compare_index(&f.repo, &f.commit, &f.index, None).unwrap()
    );
    assert_eq!(
        result
            .get("baseline_reciprocal_pairs")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert_eq!(
        result
            .get("proposed_concept_pairs")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert_eq!(result.get("unchanged_pairs").unwrap().integer().unwrap(), 1);
    assert_eq!(
        result.get("concept_only_pairs").unwrap().integer().unwrap(),
        0
    );
    assert_eq!(
        result
            .get("baseline_only_pairs")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
    assert_eq!(
        result.get("decision").unwrap().str().unwrap(),
        "DO_NOT_ENABLE_BY_DEFAULT"
    );
    assert_eq!(
        result.get("review_status").unwrap().str().unwrap(),
        "UNJUDGED"
    );
    assert_eq!(result.get("architectural_precision").unwrap(), &Value::Null);
    assert_eq!(
        result.get("held_out_independent_oracle").unwrap(),
        &Value::Bool(false)
    );
    assert_eq!(result, f.cli("scip-value", &[]));
}

#[test]
fn one_way_native_scip_evidence_has_no_new_concept_value() {
    let f = Fixture::new();
    f.write_index(false, false);
    let result = f.cli("scip-value", &[]);
    assert_eq!(
        result
            .get("baseline_reciprocal_pairs")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
    assert_eq!(
        result
            .get("proposed_concept_pairs")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
    assert_eq!(result.get("unchanged_pairs").unwrap().integer().unwrap(), 0);
}

#[test]
fn value_gate_refuses_stale_source_and_wrong_native_index_binding() {
    let f = Fixture::new();
    let mut baseline = scip_refactor::suggest(&f.repo, &f.commit, &f.index).unwrap();
    let batch = concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).unwrap();
    let model = concepts_compile::compile(&batch).unwrap();
    baseline
        .entries_mut()
        .unwrap()
        .insert("source_commit".into(), Value::string("f".repeat(40)));
    assert!(concepts_value::compare(&baseline, &batch, &model).is_err());
    let mut baseline = scip_refactor::suggest(&f.repo, &f.commit, &f.index).unwrap();
    baseline
        .entries_mut()
        .unwrap()
        .insert("index_sha256".into(), Value::string("e".repeat(64)));
    assert!(concepts_value::compare(&baseline, &batch, &model).is_err());
    let mut model = model;
    let claims = model.entries_mut().unwrap().get_mut("claims").unwrap();
    if let Value::Array(items) = claims {
        items[0]
            .entries_mut()
            .unwrap()
            .insert("behavioral_verification".into(), Value::string("PROVEN"));
    }
    assert!(
        concepts_value::compare(
            &scip_refactor::suggest(&f.repo, &f.commit, &f.index).unwrap(),
            &batch,
            &model
        )
        .is_err()
    );
}

fn ontology_draft(batch: &Value) -> Value {
    let references = batch.get("observations").unwrap().array_ref().unwrap();
    let first = references[0].get("observation_id").unwrap().clone();
    let second = references[1].get("observation_id").unwrap().clone();
    let member = |path: &str, reference: Value| {
        obj(&[
            ("path", Value::string(path)),
            ("observations", Value::array([reference])),
        ])
    };
    obj(&[
        (
            "schema",
            Value::string("excession.concepts-ontology-proposal.v1"),
        ),
        ("source_commit", batch.get("source_commit").unwrap().clone()),
        ("source_tree", batch.get("source_tree").unwrap().clone()),
        ("batch_id", batch.get("batch_id").unwrap().clone()),
        ("author_label", Value::string("agent-draft-unverified")),
        (
            "concepts",
            Value::array([obj(&[
                ("key", Value::string("cross-language-feature")),
                ("name", Value::string("Cross-language feature boundary")),
                (
                    "definition",
                    Value::string(
                        "The Rust and Python files might participate in a shared feature; source inspection is needed.",
                    ),
                ),
                (
                    "rationale",
                    Value::string(
                        "Distinct SCIP symbol occurrences connect these files in both directions.",
                    ),
                ),
                ("aliases", Value::array(vec![])),
                (
                    "members",
                    Value::array([
                        member("alpha.rs", first.clone()),
                        member("beta.py", second.clone()),
                    ]),
                ),
            ])]),
        ),
        (
            "relations",
            Value::array([obj(&[
                ("predicate", Value::string("SHARES_INTERFACE")),
                (
                    "paths",
                    Value::array([Value::string("alpha.rs"), Value::string("beta.py")]),
                ),
                ("observations", Value::array([first])),
                (
                    "rationale",
                    Value::string("Source-bound symbol reference, interface remains unverified."),
                ),
            ])]),
        ),
    ])
}

#[test]
fn llm_creates_a_new_ontology_rather_than_a_reciprocal_pair_rule() {
    let f = Fixture::new();
    let batch = concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).unwrap();
    let draft = ontology_draft(&batch);
    let artifact = concepts_author::preview(&f.repo, &f.commit, &f.index, None, &draft).unwrap();
    let model = artifact.get("model").unwrap();
    assert_eq!(model.get("concepts").unwrap().array_ref().unwrap().len(), 1);
    assert_eq!(model.get("claims").unwrap().array_ref().unwrap().len(), 3);
    let concept = &model.get("concepts").unwrap().array_ref().unwrap()[0];
    assert_eq!(
        concept.get("origin").unwrap().str().unwrap(),
        "LLM_PROPOSAL"
    );
    assert_eq!(
        concept.get("review_status").unwrap().str().unwrap(),
        "UNREVIEWED"
    );
    assert_eq!(
        concept
            .get("membership_claims")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(model.get("subjects").unwrap().array_ref().unwrap().len(), 2);
    for claim in model.get("claims").unwrap().array_ref().unwrap() {
        assert_eq!(claim.get("assessment").unwrap().str().unwrap(), "PROPOSED");
        assert_eq!(
            claim.get("behavioral_verification").unwrap().str().unwrap(),
            "UNTESTED"
        );
    }

    let db = excession::sqlite::Db::open(&f.db).unwrap();
    let published =
        concepts_author::publish(&db, &f.repo, &f.commit, &f.index, None, &draft).unwrap();
    assert_eq!(
        published.get("status").unwrap().str().unwrap(),
        "PUBLISHED_UNREVIEWED"
    );
    let id = published.get("snapshot_id").unwrap().str().unwrap();
    let accepted =
        concepts_author::audit(&db, &f.repo, &f.commit, &f.index, None, &draft, id).unwrap();
    assert_eq!(
        accepted.get("status").unwrap().str().unwrap(),
        "LLM_PROPOSAL_REPLAYED_UNREVIEWED"
    );
    assert_eq!(
        published,
        concepts_author::publish(&db, &f.repo, &f.commit, &f.index, None, &draft).unwrap()
    );

    let original = concepts_compile::compile(&batch).unwrap();
    assert_ne!(
        original.get("model_id").unwrap(),
        model.get("model_id").unwrap()
    );
}

#[test]
fn llm_ontology_refuses_invented_observations_missing_files_and_authority() {
    let f = Fixture::new();
    let batch = concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).unwrap();
    let baseline = ontology_draft(&batch);

    let mut stale = baseline.clone();
    stale
        .entries_mut()
        .unwrap()
        .insert("source_commit".into(), Value::string("f".repeat(40)));
    assert!(concepts_author::model(&batch, &stale).is_err());
    let mut invented = baseline.clone();
    let concepts = invented.entries_mut().unwrap().get_mut("concepts").unwrap();
    if let Value::Array(items) = concepts {
        let members = items[0].entries_mut().unwrap().get_mut("members").unwrap();
        if let Value::Array(files) = members {
            files[0].entries_mut().unwrap().insert(
                "observations".into(),
                Value::array([Value::string("e".repeat(64))]),
            );
        }
    }
    assert!(concepts_author::model(&batch, &invented).is_err());

    let mut missing = baseline.clone();
    let concepts = missing.entries_mut().unwrap().get_mut("concepts").unwrap();
    if let Value::Array(items) = concepts {
        let members = items[0].entries_mut().unwrap().get_mut("members").unwrap();
        if let Value::Array(files) = members {
            files[0]
                .entries_mut()
                .unwrap()
                .insert("path".into(), Value::string("absent.rs"));
        }
    }
    assert!(concepts_author::model(&batch, &missing).is_err());

    let mut illegal = baseline.clone();
    let concepts = illegal.entries_mut().unwrap().get_mut("concepts").unwrap();
    if let Value::Array(items) = concepts {
        items[0]
            .entries_mut()
            .unwrap()
            .insert("review_status".into(), Value::string("ACCEPTED"));
    }
    assert!(concepts_author::model(&batch, &illegal).is_err());
}

#[test]
fn llm_tool_context_shows_genuine_pinned_source_and_observation_ids() {
    let f = Fixture::new();
    let context =
        concepts_author::context(&f.repo, &f.commit, &f.index, None, "alpha.rs", 1).unwrap();
    assert_eq!(
        context
            .get("source")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap(),
        "alpha.rs"
    );
    assert!(
        context
            .get("matching_observation_count")
            .unwrap()
            .integer()
            .unwrap()
            > 0
    );
    assert!(
        !context
            .get("observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        context.get("coverage").unwrap().str().unwrap(),
        "PARTIAL_NATIVE_INDEX_ONLY"
    );
    let snippet = context.get("excerpt").unwrap().array_ref().unwrap();
    assert_eq!(snippet[0].get("line").unwrap().integer().unwrap(), 1);
    assert!(
        snippet[0]
            .get("text")
            .unwrap()
            .str()
            .unwrap()
            .contains("fn alpha")
    );
    assert!(concepts_author::context(&f.repo, &f.commit, &f.index, None, "../escape", 1).is_err());
    assert!(concepts_author::context(&f.repo, &f.commit, &f.index, None, "absent.py", 1).is_err());
}

#[test]
fn local_mcp_agent_lists_bounded_tools_and_can_preview_without_write_permission() {
    let f = Fixture::new();
    let batch = concepts_scip::normalize_index(&f.repo, &f.commit, &f.index, None).unwrap();
    let draft = ontology_draft(&batch);
    let mut child = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args([
            "mcp",
            "serve",
            "--repo",
            f.repo.to_str().unwrap(),
            "--rev",
            &f.commit,
            "--index",
            f.index.to_str().unwrap(),
            "--db",
            f.db.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        obj(&[
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(1)),
            ("method", Value::string("initialize")),
        ]),
        obj(&[
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(2)),
            ("method", Value::string("tools/list")),
        ]),
        obj(&[
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(3)),
            ("method", Value::string("tools/call")),
            (
                "params",
                obj(&[
                    ("name", Value::string("inspect_code_evidence")),
                    ("arguments", obj(&[("path", Value::string("alpha.rs"))])),
                ]),
            ),
        ]),
        obj(&[
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(4)),
            ("method", Value::string("tools/call")),
            (
                "params",
                obj(&[
                    ("name", Value::string("draft_ontology")),
                    ("arguments", obj(&[("proposal", draft.clone())])),
                ]),
            ),
        ]),
        obj(&[
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(5)),
            ("method", Value::string("tools/call")),
            (
                "params",
                obj(&[
                    ("name", Value::string("publish_ontology")),
                    ("arguments", obj(&[("proposal", draft)])),
                ]),
            ),
        ]),
    ];
    {
        let mut stdin = child.stdin.take().unwrap();
        for message in messages {
            writeln!(stdin, "{}", message.canonical()).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout).unwrap();
    let lines = responses
        .lines()
        .map(|s| json::parse(s.as_bytes()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    assert_eq!(
        lines[0]
            .get("result")
            .unwrap()
            .get("protocolVersion")
            .unwrap()
            .str()
            .unwrap(),
        "2025-06-18"
    );
    assert_eq!(
        lines[1]
            .get("result")
            .unwrap()
            .get("tools")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(
        lines[2].get("result").unwrap().get("isError").unwrap(),
        &Value::Bool(false)
    );
    assert_eq!(
        lines[3].get("result").unwrap().get("isError").unwrap(),
        &Value::Bool(false)
    );
    assert_eq!(
        lines[3]
            .get("result")
            .unwrap()
            .get("structuredContent")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "UNREVIEWED_LLM_PROPOSAL"
    );
    assert_eq!(
        lines[4].get("result").unwrap().get("isError").unwrap(),
        &Value::Bool(true)
    );
}

#[test]
fn ontology_source_search_is_deterministic_and_explicitly_partial() {
    let f = Fixture::new();
    let results = concepts_author::list_sources(
        &f.repo, &f.commit, &f.index, None, ".rs", 0,
    ).unwrap();
    assert_eq!(results.get("total_matched").unwrap().integer().unwrap(), 1);
    assert_eq!(results.get("sources").unwrap().array_ref().unwrap().len(), 1);
    assert_eq!(results.get("next_offset").unwrap(), &Value::Null);
    assert_eq!(results.get("coverage").unwrap().str().unwrap(), "PARTIAL_NATIVE_INDEX_ONLY");
    assert_eq!(results, concepts_author::list_sources(
        &f.repo, &f.commit, &f.index, None, ".rs", 0,
    ).unwrap());
    assert!(concepts_author::list_sources(
        &f.repo, &f.commit, &f.index, None, "", 1_000_001,
    ).is_err());
}
