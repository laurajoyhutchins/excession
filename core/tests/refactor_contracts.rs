//! Controlled Git and SCIP inputs for native refactor suggestions.
//! These are hypotheses, never evidence of independently observed behavior.
use excession::json::{self, Value, obj};
use excession::sqlite::Db;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}
fn ok(out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn denied(out: Output) {
    assert!(!out.status.success(), "unexpectedly accepted");
    assert!(out.stdout.is_empty());
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(args: &[&str]) -> Value {
    json::parse(&ok(cli(args))).unwrap()
}
fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).unwrap().str().unwrap()
}
fn items<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).unwrap().array_ref().unwrap()
}
fn edit<'a>(v: &'a mut Value, k: &str) -> &'a mut Value {
    v.entries_mut().unwrap().get_mut(k).unwrap()
}
fn item(v: &mut Value, n: usize) -> &mut Value {
    match v {
        Value::Array(xs) => &mut xs[n],
        _ => panic!("expected array"),
    }
}
struct Fixture {
    tmp: PathBuf,
    repo: PathBuf,
    commit: String,
    db: PathBuf,
    index: PathBuf,
    report: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let tmp = std::env::temp_dir().join(format!(
            "excession-refactor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = tmp.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let path = repo.to_str().unwrap();
        ok(run("git", &["-C", path, "init", "-q"]));
        if mode == "python" {
            fs::create_dir_all(repo.join("pkg")).unwrap();
            fs::write(repo.join("pkg/__init__.py"), "").unwrap();
            fs::write(
                repo.join("pkg/a.py"),
                "from pkg import b\n\ndef shared(value):\n    return value + 1\n",
            )
            .unwrap();
            fs::write(
                repo.join("pkg/b.py"),
                "from pkg import a\n\ndef shared(value):\n    return value + 1\n",
            )
            .unwrap();
        } else if mode == "javascript" {
            fs::create_dir_all(repo.join("web")).unwrap();
            fs::write(
                repo.join("web/a.js"),
                "import './b.js';\nexport function shared(value) { return value + 1; }\n",
            )
            .unwrap();
            fs::write(
                repo.join("web/b.js"),
                "import './a.js';\nexport function shared(value) { return value + 1; }\n",
            )
            .unwrap();
        } else {
            fs::write(repo.join("lib.rs"), "pub fn emit() {}\n").unwrap();
            fs::create_dir_all(repo.join("core/src")).unwrap();
            fs::write(repo.join("core/src/lib.rs"), "pub fn emit() {}\n").unwrap();
            fs::write(repo.join("app.py"), "def parse():\n    return 1\n").unwrap();
        }
        ok(run("git", &["-C", path, "add", "."]));
        ok(run(
            "git",
            &[
                "-C",
                path,
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "controlled source",
            ],
        ));
        let commit = String::from_utf8(ok(run("git", &["-C", path, "rev-parse", "HEAD"])))
            .unwrap()
            .trim()
            .to_owned();
        let db = tmp.join("index.sqlite");
        if mode == "python" || mode == "javascript" {
            ok(cli(&[
                "index",
                "--repo",
                path,
                "--db",
                db.to_str().unwrap(),
                "--name",
                "fixture",
                "--rev",
                &commit,
                "--roots",
                if mode == "python" { "pkg" } else { "web" },
            ]));
        }
        let index = tmp.join("scip.json");
        let report = tmp.join("report.json");
        Self {
            tmp,
            repo,
            commit,
            db,
            index,
            report,
        }
    }
    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn suggest(&self) -> Value {
        data(&[
            "refactor",
            "suggest",
            "--db",
            self.db.to_str().unwrap(),
            "--name",
            "fixture",
            "--rev",
            &self.commit,
        ])
    }
    fn audit(&self) -> Output {
        cli(&[
            "refactor",
            "audit",
            "--db",
            self.db.to_str().unwrap(),
            "--name",
            "fixture",
            "--rev",
            &self.commit,
            "--report",
            self.report.to_str().unwrap(),
        ])
    }
    fn scip_suggest(&self, index: &Value) -> Output {
        fs::write(&self.index, index.canonical()).unwrap();
        cli(&[
            "refactor",
            "scip-suggest",
            "--repo",
            self.repo(),
            "--rev",
            &self.commit,
            "--index",
            self.index.to_str().unwrap(),
        ])
    }
    fn scip_audit(&self) -> Output {
        cli(&[
            "refactor",
            "scip-audit",
            "--repo",
            self.repo(),
            "--rev",
            &self.commit,
            "--index",
            self.index.to_str().unwrap(),
            "--report",
            self.report.to_str().unwrap(),
        ])
    }
}
fn scip_index() -> Value {
    json::parse(br#"{
        "metadata":{"tool_info":{"name":"controlled-scip-fixture","version":"0.1"}},
        "documents":[
            {"relative_path":"lib.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,11],"symbol":"scip rust cargo example 0.1.0 api/emit().","symbol_roles":1},
                {"range":[0,7,11],"symbol":"scip python pypi example 0.1.0 app/parse()."}]},
            {"relative_path":"app.py","language":"python","position_encoding":1,
              "occurrences":[
                {"range":[0,4,9],"symbol":"scip python pypi example 0.1.0 app/parse().","symbol_roles":1},
                {"range":[0,4,9],"symbol":"scip rust cargo example 0.1.0 api/emit()."}]}
        ]
    }"#).unwrap()
}
#[test]
fn generic_native_edit_replay_is_bounded_and_unverified_for_python_and_javascript() {
    for language in ["python", "javascript"] {
        let f = Fixture::new(language);
        let plan = data(&[
            "improve",
            "plan",
            "--producer",
            "indexed",
            "--db",
            f.db.to_str().unwrap(),
            "--name",
            "fixture",
            "--rev",
            &f.commit,
        ]);
        let candidate = items(&plan, "candidates")
            .iter()
            .find(|item| {
                items(&item.get("finding").unwrap(), "locations")
                    .iter()
                    .any(|loc| {
                        let path = str_at(loc, "path");
                        fs::read_to_string(f.repo.join(path))
                            .unwrap()
                            .contains("shared")
                    })
            })
            .unwrap();
        let candidate_id = str_at(candidate, "candidate_id");
        let path = items(candidate.get("finding").unwrap(), "locations")
            .iter()
            .map(|loc| str_at(loc, "path"))
            .find(|path| {
                fs::read_to_string(f.repo.join(path))
                    .unwrap()
                    .contains("shared")
            })
            .unwrap();
        let original = fs::read_to_string(f.repo.join(path)).unwrap();
        let start = original.find("shared").unwrap();
        let blob_name = format!("{}:{path}", f.commit);
        let blob = String::from_utf8(ok(run("git", &["-C", f.repo(), "rev-parse", &blob_name])))
            .unwrap()
            .trim()
            .to_owned();
        let edit = obj(&[
            ("path", Value::string(path)),
            ("original_blob", Value::string(&blob)),
            ("start_byte", Value::number(start)),
            ("end_byte", Value::number(start + 6)),
            ("replacement", Value::string("shared_v2")),
        ]);
        let native = obj(&[
            (
                "schema",
                Value::string("excession.improvement-native-edits.v1"),
            ),
            (
                "native_producer",
                Value::string("untrusted-controlled-fixture"),
            ),
            ("source_commit", plan.get("source_commit").unwrap().clone()),
            ("source_tree", plan.get("source_tree").unwrap().clone()),
            ("improvement_plan_id", plan.get("plan_id").unwrap().clone()),
            ("candidate_id", Value::string(candidate_id)),
            ("edits", Value::array([edit])),
        ])
        .stamp_id("edit_plan_id")
        .unwrap();
        let plan_path = f.tmp.join("improve-plan.json");
        let edit_path = f.tmp.join("native-edits.json");
        fs::write(&plan_path, plan.canonical()).unwrap();
        fs::write(&edit_path, native.canonical()).unwrap();
        let output = f.tmp.join("candidate");
        let arguments = [
            "improve",
            "replay",
            "--repo",
            f.repo(),
            "--rev",
            &f.commit,
            "--plan",
            plan_path.to_str().unwrap(),
            "--edits",
            edit_path.to_str().unwrap(),
        ];
        let preview = data(&arguments);
        assert_eq!(str_at(&preview, "status"), "UNVERIFIED_EDIT_PREVIEW");
        assert_eq!(preview.get("source_changes").unwrap().integer().unwrap(), 1);
        assert!(!preview.get("ready_to_merge").unwrap().bool().unwrap());
        assert!(!output.exists());
        let applied = data(&[
            "improve",
            "replay",
            "--repo",
            f.repo(),
            "--rev",
            &f.commit,
            "--plan",
            plan_path.to_str().unwrap(),
            "--edits",
            edit_path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]);
        assert_eq!(str_at(&applied, "status"), "APPLIED_UNVERIFIED");
        assert_eq!(
            applied.get("edit_plan_id").unwrap(),
            preview.get("edit_plan_id").unwrap()
        );
        assert_eq!(fs::read_to_string(f.repo.join(path)).unwrap(), original);
        assert!(
            fs::read_to_string(output.join(path))
                .unwrap()
                .contains("shared_v2")
        );
        assert_eq!(
            String::from_utf8(ok(run("git", &["-C", f.repo(), "rev-parse", "HEAD",])))
                .unwrap()
                .trim(),
            f.commit
        );
        assert!(
            !applied
                .get("automatic_deletion_authorized")
                .unwrap()
                .bool()
                .unwrap()
        );
        assert!(items(&applied, "obligations").iter().any(|obligation| {
            obligation.str().unwrap() == "NATIVE_PRODUCER_AUTHENTICITY_UNVERIFIED"
        }));
        // Applying twice must not reuse an existing destination.
        denied(cli(&[
            "improve",
            "replay",
            "--repo",
            f.repo(),
            "--rev",
            &f.commit,
            "--plan",
            plan_path.to_str().unwrap(),
            "--edits",
            edit_path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]));
        // An altered blob hash remains forbidden even with a valid new
        // unsigned content digest.
        let mut tampered = native.clone();
        let mut forged = tampered.get("edits").unwrap().array_ref().unwrap()[0].clone();
        forged.entries_mut().unwrap().insert(
            "original_blob".into(),
            Value::string("0000000000000000000000000000000000000000"),
        );
        tampered
            .entries_mut()
            .unwrap()
            .insert("edits".into(), Value::array([forged]));
        tampered.entries_mut().unwrap().remove("edit_plan_id");
        fs::write(
            &edit_path,
            tampered.stamp_id("edit_plan_id").unwrap().canonical(),
        )
        .unwrap();
        denied(cli(&arguments));
    }
}

#[test]
fn indexed_javascript_improvement_preserves_language_specific_observations() {
    let f = Fixture::new("javascript");
    let native = f.suggest();
    let candidates = items(&native, "candidates");
    assert!(!candidates.is_empty(), "expected real JS parser findings");
    assert!(candidates.iter().any(|item| {
        str_at(item, "kind") == "dependency_cycle"
            || str_at(item, "kind") == "duplicate_body_fingerprint"
    }));

    let plan = data(&[
        "improve",
        "plan",
        "--producer",
        "indexed",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--rev",
        &f.commit,
    ]);
    assert_eq!(str_at(&plan, "producer"), "indexed");
    assert_eq!(
        plan.get("candidate_count").unwrap().integer().unwrap(),
        candidates.len() as i64
    );
    assert!(
        items(&plan, "candidates")
            .iter()
            .all(|candidate| { str_at(candidate, "edit_capability") == "NO_NATIVE_EDIT_PRODUCER" })
    );
    assert!(
        !plan
            .get("automatic_deletion_authorized")
            .unwrap()
            .bool()
            .unwrap()
    );
}

#[test]
fn indexed_python_improvement_uses_the_same_evidence_contract_as_scip() {
    let f = Fixture::new("python");
    let plan = data(&[
        "improve",
        "plan",
        "--producer",
        "indexed",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--rev",
        &f.commit,
    ]);
    assert_eq!(str_at(&plan, "producer"), "indexed");
    assert_eq!(plan.get("candidate_count").unwrap().integer().unwrap(), 2);
    assert!(
        items(&plan, "candidates")
            .iter()
            .all(|candidate| { str_at(candidate, "edit_capability") == "NO_NATIVE_EDIT_PRODUCER" })
    );
    assert!(!plan.get("ready_to_merge").unwrap().bool().unwrap());

    let saved = f.tmp.join("improvement.json");
    fs::write(&saved, plan.canonical()).unwrap();
    // Remove one import edge but deliberately retain a duplicate function:
    // fewer source-bound findings is an observation, not proof of equivalence.
    fs::write(
        f.repo.join("pkg/b.py"),
        "def shared(value):\n    return value + 1\n",
    )
    .unwrap();
    ok(run("git", &["-C", f.repo(), "add", "pkg/b.py"]));
    ok(run(
        "git",
        &[
            "-C",
            f.repo(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "remove one static import candidate",
        ],
    ));
    let head = String::from_utf8(ok(run("git", &["-C", f.repo(), "rev-parse", "HEAD"])))
        .unwrap()
        .trim()
        .to_owned();
    ok(cli(&[
        "index",
        "--repo",
        f.repo(),
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--rev",
        &head,
        "--roots",
        "pkg",
    ]));

    let args = [
        "improve",
        "evaluate",
        "--producer",
        "indexed",
        "--repo",
        f.repo(),
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--base",
        &f.commit,
        "--candidate",
        &head,
        "--plan",
        saved.to_str().unwrap(),
    ];
    let evaluated = data(&args);
    assert_eq!(str_at(&evaluated, "producer"), "indexed");
    assert_eq!(
        str_at(&evaluated, "status"),
        "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED"
    );
    assert_eq!(
        evaluated
            .get("observed_findings_before")
            .unwrap()
            .integer()
            .unwrap(),
        2
    );
    assert_eq!(
        evaluated
            .get("observed_findings_after")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert_eq!(
        evaluated.get("source_changes").unwrap().integer().unwrap(),
        1
    );
    assert!(!evaluated.get("ready_to_merge").unwrap().bool().unwrap());
    assert!(
        !evaluated
            .get("automatic_deletion_authorized")
            .unwrap()
            .bool()
            .unwrap()
    );

    let mut invalid = plan.clone();
    invalid
        .entries_mut()
        .unwrap()
        .insert("candidate_count".into(), Value::number(0));
    fs::write(&saved, invalid.canonical()).unwrap();
    denied(cli(&args));
    denied(cli(&[
        "improve",
        "trial",
        "--producer",
        "indexed",
        "--repo",
        f.repo(),
        "--rev",
        &f.commit,
        "--index",
        "unused",
        "--report",
        "unused",
        "--plan",
        saved.to_str().unwrap(),
        "--candidate",
        "untrusted",
        "--trusted-local",
        "true",
    ]));
}

#[test]
fn source_bound_refactor_cycle_and_duplicate_body_replay() {
    let f = Fixture::new("python");
    let report = f.suggest();
    assert_eq!(
        str_at(&report, "schema"),
        "excession.refactor-candidates.v1"
    );
    assert_eq!(str_at(&report, "source_commit"), f.commit);
    assert_eq!(str_at(&report, "status"), "UNVERIFIED_REFACTOR_HYPOTHESES");
    let candidates = items(&report, "candidates");
    assert_eq!(candidates.len(), 2);
    let cycle = candidates
        .iter()
        .find(|c| str_at(c, "kind") == "dependency_cycle")
        .unwrap();
    let names = items(cycle, "modules")
        .iter()
        .map(|v| str_at(v, "name"))
        .collect::<Vec<_>>();
    assert!(names.contains(&"pkg.a") && names.contains(&"pkg.b"));
    assert_eq!(items(cycle, "edges").len(), 2);
    let duplicate = candidates
        .iter()
        .find(|c| str_at(c, "kind") == "duplicate_body_fingerprint")
        .unwrap();
    assert_eq!(str_at(duplicate, "language"), "python");
    let symbols = items(duplicate, "symbols");
    assert_eq!(symbols.len(), 2);
    for entry in symbols {
        let path = str_at(entry, "path");
        assert!(["pkg/a.py", "pkg/b.py"].contains(&path));
        let spec = format!("{}:{path}", f.commit);
        let observed =
            String::from_utf8(ok(run("git", &["-C", f.repo(), "rev-parse", &spec]))).unwrap();
        assert_eq!(str_at(entry, "blob"), observed.trim());
    }
    assert_eq!(f.suggest(), report);
    fs::write(&f.report, report.canonical()).unwrap();
    let receipt = json::parse(&ok(f.audit())).unwrap();
    assert_eq!(
        receipt.get("report_id").unwrap(),
        report.get("report_id").unwrap()
    );
    assert!(
        !receipt
            .get("semantic_reparse_performed")
            .unwrap()
            .bool()
            .unwrap()
    );
}
#[test]
fn tampered_refactor_advice_and_index_relations_are_refused() {
    let f = Fixture::new("python");
    let mut report = f.suggest();
    let mut candidates = items(&report, "candidates").to_vec();
    candidates[0]
        .entries_mut()
        .unwrap()
        .insert("action".into(), Value::string("forged advice"));
    report
        .entries_mut()
        .unwrap()
        .insert("candidates".into(), Value::array(candidates));
    fs::write(&f.report, report.canonical()).unwrap();
    denied(f.audit());
    fs::write(&f.report, f.suggest().canonical()).unwrap();
    Db::open(&f.db)
        .unwrap()
        .execute(
            "DELETE FROM dependency WHERE source_name=? AND target_name=?",
            &[
                excession::sqlite::Param::Text("pkg.a"),
                excession::sqlite::Param::Text("pkg.b"),
            ],
        )
        .unwrap();
    denied(f.audit());
}
#[test]
fn refactor_requires_indexed_revision_and_unique_flags() {
    let f = Fixture::new("python");
    fs::write(f.repo.join("extra.txt"), "new commit\n").unwrap();
    ok(run("git", &["-C", f.repo(), "add", "."]));
    ok(run(
        "git",
        &[
            "-C",
            f.repo(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "next",
        ],
    ));
    assert_eq!(str_at(&f.suggest(), "source_commit"), f.commit);
    denied(cli(&[
        "refactor",
        "suggest",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--rev",
        "HEAD",
    ]));
    denied(cli(&[
        "refactor",
        "suggest",
        "--db",
        f.db.to_str().unwrap(),
        "--name",
        "fixture",
        "--rev",
        &f.commit,
        "--rev",
        &f.commit,
    ]));
}
#[test]
fn scip_cross_language_references_are_unverified_and_source_bound() {
    let f = Fixture::new("scip");
    let index = scip_index();
    let report = json::parse(&ok(f.scip_suggest(&index))).unwrap();
    assert_eq!(str_at(&report, "schema"), "excession.scip-refactor.v1");
    assert_eq!(str_at(&report, "source_commit"), f.commit);
    assert_eq!(
        items(report.get("coverage").unwrap(), "languages"),
        &[Value::string("python"), Value::string("rust")]
    );
    let coverage = report.get("coverage").unwrap();
    assert_eq!(
        coverage
            .get("matched_cross_file_references")
            .unwrap()
            .integer()
            .unwrap(),
        2
    );
    assert_eq!(
        coverage
            .get("unresolved_references")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
    let candidates = items(&report, "candidates");
    assert_eq!(
        candidates
            .iter()
            .filter(|c| str_at(c, "kind") == "reciprocal_symbol_references")
            .count(),
        1
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|c| str_at(c, "kind") == "cross_language_symbol_boundary")
            .count(),
        2
    );
    assert!(
        candidates
            .iter()
            .all(|c| str_at(c, "status") == "UNVERIFIED_REFACTOR_HYPOTHESIS")
    );
    fs::write(&f.report, report.canonical()).unwrap();
    let receipt = json::parse(&ok(f.scip_audit())).unwrap();
    assert_eq!(
        receipt.get("report_id").unwrap(),
        report.get("report_id").unwrap()
    );
    assert!(
        !receipt
            .get("indexer_independently_executed")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert_eq!(json::parse(&ok(f.scip_suggest(&index))).unwrap(), report);
}
#[test]
fn scip_omitted_language_is_not_cross_language_evidence() {
    let fixture = Fixture::new("scip");
    let mut index = scip_index();
    let documents = edit(&mut index, "documents");
    if let Value::Array(values) = documents {
        for doc in values {
            doc.entries_mut().unwrap().remove("language");
        }
    } else {
        panic!("SCIP documents must be an array");
    }
    let report = json::parse(&ok(fixture.scip_suggest(&index))).unwrap();
    assert_eq!(
        items(report.get("coverage").unwrap(), "languages"),
        &[Value::string("unspecified")]
    );
    assert_eq!(
        items(&report, "candidates")
            .iter()
            .filter(|candidate| str_at(candidate, "kind") == "cross_language_symbol_boundary")
            .count(),
        0
    );
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("matched_cross_file_references")
            .unwrap()
            .integer()
            .unwrap(),
        2
    );
}

#[test]
fn scip_candidate_edges_require_exact_symbol_identity() {
    let f = Fixture::new("scip");
    let mut index = scip_index();
    let documents = edit(&mut index, "documents");
    let occurrences = edit(item(documents, 1), "occurrences");
    edit(item(occurrences, 1), "symbol").clone_from(&Value::string(
        "scip rust cargo different 0.1.0 api/emit().",
    ));
    let report = json::parse(&ok(f.scip_suggest(&index))).unwrap();
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("unresolved_references")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    let candidates = items(&report, "candidates");
    assert_eq!(
        candidates
            .iter()
            .filter(|c| str_at(c, "kind") == "cross_language_symbol_boundary")
            .count(),
        1
    );
    assert!(
        !candidates
            .iter()
            .any(|c| str_at(c, "kind") == "reciprocal_symbol_references")
    );
}
#[test]
fn scip_forged_reports_paths_ranges_and_sources_are_refused() {
    let f = Fixture::new("scip");
    let index = scip_index();
    let mut report = json::parse(&ok(f.scip_suggest(&index))).unwrap();
    edit(
        edit(&mut report, "coverage"),
        "matched_cross_file_references",
    )
    .clone_from(&Value::number(200));
    fs::write(&f.report, report.canonical()).unwrap();
    denied(f.scip_audit());
    let mut escaped = scip_index();
    edit(item(edit(&mut escaped, "documents"), 0), "relative_path")
        .clone_from(&Value::string("../escape.rs"));
    denied(f.scip_suggest(&escaped));
    edit(item(edit(&mut escaped, "documents"), 0), "relative_path")
        .clone_from(&Value::string("lib.rs"));
    edit(
        item(
            edit(item(edit(&mut escaped, "documents"), 0), "occurrences"),
            0,
        ),
        "range",
    )
    .clone_from(&Value::array([
        Value::number(90),
        Value::number(0),
        Value::number(1),
    ]));
    denied(f.scip_suggest(&escaped));
    let mut textual = scip_index();
    item(edit(&mut textual, "documents"), 0)
        .entries_mut()
        .unwrap()
        .insert("text".into(), Value::string("forged source"));
    denied(f.scip_suggest(&textual));
}
#[test]
fn changed_scip_index_invalidates_audit() {
    let f = Fixture::new("scip");
    let mut index = scip_index();
    let report = json::parse(&ok(f.scip_suggest(&index))).unwrap();
    fs::write(&f.report, report.canonical()).unwrap();
    edit(
        item(
            edit(item(edit(&mut index, "documents"), 1), "occurrences"),
            1,
        ),
        "symbol",
    )
    .clone_from(&Value::string(
        "scip python pypi example 0.1.0 app/parse().",
    ));
    fs::write(&f.index, index.canonical()).unwrap();
    denied(f.scip_audit());
}

#[test]
fn scip_nested_project_root_and_prefix_are_source_bound() {
    let f = Fixture::new("scip");
    let mut index = json::parse(
        br#"{
        "metadata":{"tool_info":{"name":"rust-analyzer","version":"1.99.1"}},
        "documents":[{"relative_path":"src/lib.rs","language":"rust",
          "position_encoding":1,"occurrences":[
          {"range":[0,7,11],"symbol":"scip rust cargo example 0.1.0 api/emit().",
           "symbol_roles":1}]}]
    }"#,
    )
    .unwrap();
    let project_root = format!("file://{}", f.repo.join("core").display());
    edit(&mut index, "metadata")
        .entries_mut()
        .unwrap()
        .insert("project_root".into(), Value::string(&project_root));
    fs::write(&f.index, index.canonical()).unwrap();
    let args = [
        "refactor",
        "scip-suggest",
        "--repo",
        f.repo(),
        "--rev",
        &f.commit,
        "--index",
        f.index.to_str().unwrap(),
        "--source-prefix",
        "core",
    ];
    let report = data(&args);
    assert_eq!(str_at(&report, "source_commit"), f.commit);
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("indexed_documents")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
    assert_eq!(
        items(report.get("coverage").unwrap(), "languages"),
        &[Value::string("rust")]
    );
    fs::write(&f.report, report.canonical()).unwrap();
    let receipt = data(&[
        "refactor",
        "scip-audit",
        "--repo",
        f.repo(),
        "--rev",
        &f.commit,
        "--index",
        f.index.to_str().unwrap(),
        "--report",
        f.report.to_str().unwrap(),
        "--source-prefix",
        "core",
    ]);
    assert_eq!(
        receipt.get("report_id").unwrap(),
        report.get("report_id").unwrap()
    );
    edit(&mut index, "metadata").entries_mut().unwrap().insert(
        "project_root".into(),
        Value::string("file:///unrelated/checkout"),
    );
    fs::write(&f.index, index.canonical()).unwrap();
    denied(cli(&args));
    edit(&mut index, "metadata")
        .entries_mut()
        .unwrap()
        .insert("project_root".into(), Value::string(&project_root));
    edit(item(edit(&mut index, "documents"), 0), "relative_path")
        .clone_from(&Value::string("../lib.rs"));
    fs::write(&f.index, index.canonical()).unwrap();
    denied(cli(&args));
}
