//! Real pinned-Git integration of SCIP and configured-build evidence.
use excession::configured_store::{self, GraphSnapshot};
use excession::git;
use excession::json::{self, Value, obj};
use excession::scip_evidence;
use excession::sqlite::{Db, Param};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn command(repo: &Path, executable: &str, args: &[&str]) -> Output {
    Command::new(executable)
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn refuse(output: Output) {
    assert!(!output.status.success(), "unexpected success");
    assert!(
        output.stdout.is_empty(),
        "partial success output on refusal"
    );
}

fn commit(repo: &Path, label: &str) -> String {
    success(command(repo, "git", &["add", "-A"]));
    success(command(
        repo,
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            label,
        ],
    ));
    success(command(repo, "git", &["rev-parse", "HEAD"]))
}

fn native_index() -> Value {
    json::parse(
        br#"{
          "metadata":{"tool_info":{"name":"fixture-scip","version":"1.0"}},
          "documents":[
            {"relative_path":"src/a.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/foo().","symbol_roles":1},
                {"range":[0,15,18],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":0}
              ]},
            {"relative_path":"src/b.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,10],"symbol":"scip rust cargo demo 0.1.0 demo/bar().","symbol_roles":1}
              ]}
          ]
        }"#,
    )
    .unwrap()
}

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    db: PathBuf,
    base: String,
    head: String,
    base_snapshot: String,
    head_snapshot: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-relations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        success(command(&repo, "git", &["init", "-q"]));
        fs::write(repo.join("src/a.rs"), "pub fn foo() { bar(); }\n").unwrap();
        fs::write(repo.join("src/b.rs"), "pub fn bar() {}\n").unwrap();
        let base = commit(&repo, "base");
        let index = root.join("scip.json");
        fs::write(&index, native_index().canonical()).unwrap();
        let db = root.join("evidence.sqlite");
        let store = Db::open(&db).unwrap();
        let base_report = scip_evidence::import(&repo, &base, &index, None, &store).unwrap();
        let base_snapshot = base_report
            .get("snapshot_id")
            .unwrap()
            .str()
            .unwrap()
            .to_owned();

        fs::write(repo.join("src/b.rs"), "pub fn bar() { 1; }\n").unwrap();
        let head = commit(&repo, "change bar");
        let head_report = scip_evidence::import(&repo, &head, &index, None, &store).unwrap();
        let head_snapshot = head_report
            .get("snapshot_id")
            .unwrap()
            .str()
            .unwrap()
            .to_owned();
        Self {
            root,
            repo,
            db,
            base,
            head,
            base_snapshot,
            head_snapshot,
        }
    }

    fn db(&self) -> &str {
        self.db.to_str().unwrap()
    }
    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn cli(&self, args: &[&str]) -> Output {
        command(&self.repo, env!("CARGO_BIN_EXE_excession"), args)
    }

    fn graph_for_changed_source(&self) -> String {
        self.graph_for_target("demo")
    }

    fn graph_for_target(&self, target: &str) -> String {
        let store = Db::open(&self.db).unwrap();
        let blob = git::blob(&self.repo, &self.head, "src/b.rs").unwrap();
        let tree = git::rev(&self.repo, &format!("{}^{{tree}}", self.head)).unwrap();
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        configured_store::push_node(
            &mut nodes,
            format!("target:{target}"),
            "configured-target",
            obj(&[("name", Value::string(target))]),
        )
        .unwrap();
        configured_store::push_node(
            &mut nodes,
            "source:src/b.rs".into(),
            "git-source",
            obj(&[
                ("path", Value::string("src/b.rs")),
                ("blob", Value::string(blob)),
            ]),
        )
        .unwrap();
        configured_store::push_edge(
            &mut edges,
            &format!("target:{target}"),
            "source:src/b.rs",
            "TARGET_SOURCE",
            "CMAKE_CONFIGURED_GIT_VERIFIED",
            "fixture-native-codemodel",
        )
        .unwrap();
        let graph = obj(&[
            ("schema", Value::string("excession.configured-graph.v1")),
            ("source_commit", Value::string(&self.head)),
            ("source_tree", Value::string(tree)),
            ("target", Value::string(target)),
            ("configuration", Value::string("test")),
            ("status", Value::string("CONFIGURED_MEMBERSHIP_ONLY")),
            (
                "limitations",
                Value::array([Value::string("No build executed")]),
            ),
            ("nodes", Value::array(nodes.values().cloned())),
            ("edges", Value::array(edges.values().cloned())),
        ]);
        let id = graph.sha256();
        configured_store::persist(GraphSnapshot {
            db: &store,
            id: &id,
            commit: &self.head,
            target,
            configuration: "test",
            graph: &graph,
            nodes: &nodes,
            edges: &edges,
            observations: &[],
        })
        .unwrap();
        id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn rust_only_scip_reference_enters_analysis_and_revision_impact() {
    let f = Fixture::new();
    // Rust-only source without imported evidence is a partial report, not
    // an absence proof or a fatal dependency on Python/JavaScript analyzers.
    let empty = json::parse(success(f.cli(&["analyze", f.repo(), "--json"])).as_bytes()).unwrap();
    assert!(empty.get("relationships").unwrap().array_ref().unwrap().is_empty());
    assert_eq!(empty.get("coverage").unwrap().get("status").unwrap().str().unwrap(), "PARTIAL");
    let report = json::parse(
        success(f.cli(&[
            "analyze",
            f.repo(),
            "--rev",
            &f.head,
            "--scip-snapshot",
            &f.head_snapshot,
            "--db",
            f.db(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(report.get("source_commit").unwrap().str().unwrap(), f.head);
    assert_eq!(
        report
            .get("coverage")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "PARTIAL"
    );
    let links = report.get("relationships").unwrap().array_ref().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].get("kind").unwrap().str().unwrap(),
        "SCIP_REFERENCE"
    );
    assert_eq!(
        links[0]
            .get("source")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap(),
        "src/a.rs"
    );
    assert_eq!(
        links[0]
            .get("target")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap(),
        "src/b.rs"
    );
    assert_eq!(
        links[0].get("evidence").unwrap().array_ref().unwrap()[0]
            .get("assessment")
            .unwrap()
            .str()
            .unwrap(),
        "OBSERVED"
    );
    assert_eq!(
        links[0]
            .get("native_observation")
            .unwrap()
            .get("native_kind")
            .unwrap()
            .str()
            .unwrap(),
        "SCIP_SYMBOL_REFERENCE"
    );

    let saved = f.root.join("impact.json");
    let impacted = json::parse(
        success(f.cli(&[
            "impact",
            f.repo(),
            "--base",
            &f.base,
            "--head",
            &f.head,
            "--base-scip-snapshot",
            &f.base_snapshot,
            "--head-scip-snapshot",
            &f.head_snapshot,
            "--db",
            f.db(),
            "--output",
            saved.to_str().unwrap(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert!(
        !impacted
            .get("safe_to_skip_unlisted_tests")
            .unwrap()
            .bool()
            .unwrap()
    );
    let effects = impacted.get("effects").unwrap().array_ref().unwrap();
    assert!(effects.iter().any(|item| {
        item.get("entity")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap()
            == "src/a.rs"
            && item.get("assessment").unwrap().str().unwrap() == "CANDIDATE_PATH"
    }));
    let claim = effects
        .iter()
        .find(|item| {
            item.get("entity")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                == "src/a.rs"
        })
        .unwrap()
        .get("claim_id")
        .unwrap()
        .str()
        .unwrap();
    let explained = json::parse(
        success(f.cli(&[
            "explain",
            f.repo(),
            "--report",
            saved.to_str().unwrap(),
            "--id",
            claim,
            "--db",
            f.db(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(explained.get("claim_id").unwrap().str().unwrap(), claim);
}

#[test]
fn configured_membership_is_visible_but_never_invents_a_dependency() {
    let f = Fixture::new();
    let graph = f.graph_for_changed_source();
    let analysis = json::parse(
        success(f.cli(&[
            "analyze",
            f.repo(),
            "--rev",
            &f.head,
            "--graph-id",
            &graph,
            "--db",
            f.db(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert!(
        analysis
            .get("relationships")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    let configured = analysis
        .get("configured_observations")
        .unwrap()
        .array_ref()
        .unwrap();
    assert_eq!(configured.len(), 1);
    assert_eq!(
        configured[0].get("relation").unwrap().str().unwrap(),
        "uses"
    );
    assert_eq!(
        configured[0].get("native_kind").unwrap().str().unwrap(),
        "TARGET_SOURCE"
    );
    let impact = json::parse(
        success(f.cli(&[
            "impact",
            f.repo(),
            "--base",
            &f.base,
            "--head",
            &f.head,
            "--head-graph-id",
            &graph,
            "--db",
            f.db(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert!(
        impact
            .get("effects")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        impact
            .get("configured_observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !impact
            .get("safe_to_skip_unlisted_tests")
            .unwrap()
            .bool()
            .unwrap()
    );

    // Both producers may contribute their distinct evidence at once.
    let both = json::parse(
        success(f.cli(&[
            "analyze",
            f.repo(),
            "--rev",
            &f.head,
            "--graph-id",
            &graph,
            "--scip-snapshot",
            &f.head_snapshot,
            "--db",
            f.db(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        both.get("relationships")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        both.get("configured_observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stale_wrong_scope_and_sql_tampering_are_refused() {
    let f = Fixture::new();
    let graph = f.graph_for_changed_source();
    // Cannot transplant a head snapshot into a base revision.
    refuse(f.cli(&[
        "analyze",
        f.repo(),
        "--rev",
        &f.base,
        "--scip-snapshot",
        &f.head_snapshot,
        "--db",
        f.db(),
        "--json",
    ]));
    // No arbitrary SHA is silently treated as a valid saved observation.
    refuse(f.cli(&[
        "analyze",
        f.repo(),
        "--rev",
        &f.head,
        "--scip-snapshot",
        &"a".repeat(64),
        "--db",
        f.db(),
        "--json",
    ]));
    refuse(f.cli(&[
        "analyze",
        f.repo(),
        "--rev",
        &f.base,
        "--graph-id",
        &graph,
        "--db",
        f.db(),
        "--json",
    ]));
    let store = Db::open(&f.db).unwrap();
    store
        .execute(
            "UPDATE configured_edge SET status='FORGED' WHERE graph_id=?",
            &[Param::Text(&graph)],
        )
        .unwrap();
    refuse(f.cli(&[
        "analyze",
        f.repo(),
        "--rev",
        &f.head,
        "--graph-id",
        &graph,
        "--db",
        f.db(),
        "--json",
    ]));
}

#[test]
fn default_discovers_unique_revision_evidence_and_pins_replay() {
    let f = Fixture::new();
    let graph = f.graph_for_changed_source();
    let discovered = json::parse(success(f.cli(&[
        "inspect", "producer-candidates", "--repo", f.repo(), "--rev", &f.head, "--db", f.db(),
    ])).as_bytes()).unwrap();
    assert_eq!(discovered.get("status").unwrap().str().unwrap(), "UNAMBIGUOUS");

    let analysis = json::parse(success(f.cli(&[
        "analyze", f.repo(), "--rev", &f.head, "--db", f.db(), "--json",
    ])).as_bytes()).unwrap();
    assert_eq!(analysis.get("relationships").unwrap().array_ref().unwrap().len(), 1);
    assert_eq!(analysis.get("configured_observations").unwrap().array_ref().unwrap().len(), 1);
    let plan = analysis.get("producer_selection_plan").unwrap();
    assert_eq!(
        plan.get("scip").unwrap().array_ref().unwrap()[0]
            .get("snapshot_id").unwrap().str().unwrap(), f.head_snapshot
    );
    assert_eq!(plan.get("graphs").unwrap().array_ref().unwrap()[0].str().unwrap(), graph);

    let saved = f.root.join("default-impact.json");
    let impact = json::parse(success(f.cli(&[
        "impact", f.repo(), "--base", &f.base, "--head", &f.head,
        "--db", f.db(), "--output", saved.to_str().unwrap(), "--json",
    ])).as_bytes()).unwrap();
    let selections = impact.get("producer_selection_plan").unwrap();
    assert_eq!(
        selections.get("base").unwrap().get("scip").unwrap().array_ref().unwrap()[0]
            .get("snapshot_id").unwrap().str().unwrap(), f.base_snapshot
    );
    assert_eq!(
        selections.get("head").unwrap().get("scip").unwrap().array_ref().unwrap()[0]
            .get("snapshot_id").unwrap().str().unwrap(), f.head_snapshot
    );
    let effect = impact.get("effects").unwrap().array_ref().unwrap().iter().find(
        |x| x.get("entity").unwrap().get("path").unwrap().str().unwrap() == "src/a.rs"
    ).unwrap();
    let claim = effect.get("claim_id").unwrap().str().unwrap();
    // Import a new competing SCIP snapshot after the report was generated.
    // Explanation must replay only the original chosen identities.
    let db = Db::open(&f.db).unwrap();
    let alternate = native_index().canonical().replace(
        r#""version":"1.0""#, r#""version":"2.0""#
    );
    let index = f.root.join("late-scip.json");
    fs::write(&index, alternate).unwrap();
    scip_evidence::import(&f.repo, &f.head, &index, None, &db).unwrap();
    let explained = json::parse(success(f.cli(&[
        "explain", f.repo(), "--report", saved.to_str().unwrap(),
        "--id", claim, "--db", f.db(), "--json",
    ])).as_bytes()).unwrap();
    assert_eq!(explained.get("claim_id").unwrap().str().unwrap(), claim);
    assert!(!impact.get("safe_to_skip_unlisted_tests").unwrap().bool().unwrap());
}

#[test]
fn competing_scip_indexes_quarantine_only_overlapping_paths() {
    let f = Fixture::new();
    let graph = f.graph_for_changed_source();
    let db = Db::open(&f.db).unwrap();
    let alternate = native_index().canonical().replace(
        r#""version":"1.0""#, r#""version":"2.0""#
    );
    let index = f.root.join("duplicate-scip.json");
    fs::write(&index, alternate).unwrap();
    scip_evidence::import(&f.repo, &f.head, &index, None, &db).unwrap();

    let composed = json::parse(success(f.cli(&[
        "analyze", f.repo(), "--rev", &f.head, "--db", f.db(), "--json",
    ])).as_bytes()).unwrap();
    assert!(composed.get("relationships").unwrap().array_ref().unwrap().is_empty());
    assert_eq!(composed.get("configured_observations").unwrap().array_ref().unwrap().len(), 1);
    let plan = composed.get("producer_selection_plan").unwrap();
    assert_eq!(plan.get("scip").unwrap().array_ref().unwrap().len(), 2);
    assert_eq!(plan.get("blocked_paths").unwrap().array_ref().unwrap().len(), 2);
    let gaps = composed.get("coverage").unwrap().get("gaps").unwrap().array_ref().unwrap();
    assert!(gaps.iter().any(|g| g.get("reason").unwrap().str().unwrap() == "UNRESOLVED_RELATIONSHIP"));

    let impact = json::parse(success(f.cli(&[
        "impact", f.repo(), "--base", &f.base, "--head", &f.head,
        "--db", f.db(), "--json",
    ])).as_bytes()).unwrap();
    assert!(impact.get("effects").unwrap().array_ref().unwrap().is_empty());
    assert_eq!(impact.get("configured_observations").unwrap().array_ref().unwrap().len(), 1);
    assert!(!impact.get("safe_to_skip_unlisted_tests").unwrap().bool().unwrap());

    // Explicit narrowing still permits the originally authenticated index.
    let narrow = json::parse(success(f.cli(&[
        "analyze", f.repo(), "--rev", &f.head, "--db", f.db(),
        "--scip-snapshot", &f.head_snapshot, "--json",
    ])).as_bytes()).unwrap();
    assert_eq!(narrow.get("relationships").unwrap().array_ref().unwrap().len(), 1);
    assert_eq!(narrow.get("producer_selections").unwrap().get("scip_snapshot").unwrap()
        .str().unwrap(), f.head_snapshot);
    assert_eq!(plan.get("graphs").unwrap().array_ref().unwrap()[0].str().unwrap(), graph);
}

#[test]
fn disjoint_scip_indexes_compose_and_build_targets_remain_independent() {
    let f = Fixture::new();
    let graph_a = f.graph_for_target("demo");
    let graph_b = f.graph_for_target("other");
    assert_ne!(graph_a, graph_b);
    fs::write(f.repo.join("src/c.rs"), "pub fn cat() { dog(); }\n").unwrap();
    fs::write(f.repo.join("src/d.rs"), "pub fn dog() {}\n").unwrap();
    let revision = commit(&f.repo, "add other source pair");
    let db = Db::open(&f.db).unwrap();
    let first = f.root.join("first-scip.json");
    fs::write(&first, native_index().canonical()).unwrap();
    scip_evidence::import(&f.repo, &revision, &first, None, &db).unwrap();
    let independent = native_index().canonical()
        .replace("src/a.rs", "src/c.rs")
        .replace("src/b.rs", "src/d.rs")
        .replace("foo", "cat")
        .replace("bar", "dog");
    let second = f.root.join("second-scip.json");
    fs::write(&second, independent).unwrap();
    scip_evidence::import(&f.repo, &revision, &second, None, &db).unwrap();
    let report = json::parse(success(f.cli(&[
        "analyze", f.repo(), "--rev", &revision, "--db", f.db(), "--json",
    ])).as_bytes()).unwrap();
    let edges = report.get("relationships").unwrap().array_ref().unwrap();
    assert_eq!(edges.len(), 2);
    let plan = report.get("producer_selection_plan").unwrap();
    assert_eq!(plan.get("scip").unwrap().array_ref().unwrap().len(), 2);
    assert!(plan.get("blocked_paths").unwrap().array_ref().unwrap().is_empty());

    // Two independent graph targets are both retained when selected at the
    // same revision; they do not create source-to-source import edges.
    let f2 = Fixture::new();
    let graph_one = f2.graph_for_target("demo");
    let graph_two = f2.graph_for_target("other");
    let report2 = json::parse(success(f2.cli(&[
        "analyze", f2.repo(), "--rev", &f2.head, "--db", f2.db(), "--json",
    ])).as_bytes()).unwrap();
    let native = report2.get("configured_observations").unwrap().array_ref().unwrap();
    assert_eq!(native.len(), 2);
    assert_eq!(report2.get("relationships").unwrap().array_ref().unwrap().len(), 1);
    let ids = report2.get("producer_selection_plan").unwrap().get("graphs").unwrap()
        .array_ref().unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().any(|x| x.str().unwrap() == graph_one));
    assert!(ids.iter().any(|x| x.str().unwrap() == graph_two));
}

#[test]
fn discovery_does_not_create_missing_store_or_assume_source_completeness() {
    let f = Fixture::new();
    let missing = f.root.join("not-an-evidence-db.sqlite");
    refuse(f.cli(&[
        "inspect",
        "producer-candidates",
        "--repo",
        f.repo(),
        "--rev",
        &f.head,
        "--db",
        missing.to_str().unwrap(),
    ]));
    assert!(!missing.exists());
    let blank = f.root.join("empty.sqlite");
    let _db = Db::open(&blank).unwrap();
    let found = json::parse(
        success(f.cli(&[
            "inspect",
            "producer-candidates",
            "--repo",
            f.repo(),
            "--rev",
            &f.head,
            "--db",
            blank.to_str().unwrap(),
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        found.get("status").unwrap().str().unwrap(),
        "NO_IMPORTED_EVIDENCE"
    );
    let partial = json::parse(success(f.cli(&[
        "analyze", f.repo(), "--rev", &f.head,
        "--db", blank.to_str().unwrap(), "--json",
    ])).as_bytes()).unwrap();
    assert_eq!(partial.get("coverage").unwrap().get("status").unwrap().str().unwrap(), "PARTIAL");
    assert!(partial.get("relationships").unwrap().array_ref().unwrap().is_empty());
    assert!(partial.get("producer_selection_plan").unwrap().get("scip").unwrap()
        .array_ref().unwrap().is_empty());
    let store = Db::open(&f.db).unwrap();
    store
        .execute(
            "UPDATE evidence_receipt_v1 SET document='{}' WHERE snapshot_id=?",
            &[Param::Text(&f.head_snapshot)],
        )
        .unwrap();
    refuse(f.cli(&[
        "inspect",
        "producer-candidates",
        "--repo",
        f.repo(),
        "--rev",
        &f.head,
        "--db",
        f.db(),
    ]));
}

#[test]
fn auto_detects_native_scip_subproject_only_from_declared_project_root() {
    let f = Fixture::new();
    let nested = f.repo.join("packages/native");
    fs::create_dir_all(nested.join("src")).unwrap();
    fs::write(nested.join("src/a.rs"), "pub fn foo() { bar(); }\n").unwrap();
    fs::write(nested.join("src/b.rs"), "pub fn bar() {}\n").unwrap();
    let revision = commit(&f.repo, "nested project");
    let canonical = nested.canonicalize().unwrap();
    let root_uri = format!("file://{}", canonical.display());
    let native = native_index().canonical();
    let source = native.replace(
        r#""metadata":{"tool_info""#,
        &format!(r#""metadata":{{"project_root":"{root_uri}","tool_info""#),
    );
    assert_ne!(source, native);
    let input = f.root.join("nested-index.json");
    fs::write(&input, source).unwrap();
    let database = f.root.join("nested.sqlite");
    let db = Db::open(&database).unwrap();
    let imported =
        scip_evidence::import(&f.repo, &revision, &input, Some("packages/native"), &db).unwrap();
    let found = json::parse(
        success(f.cli(&[
            "inspect",
            "producer-candidates",
            "--repo",
            f.repo(),
            "--rev",
            &revision,
            "--db",
            database.to_str().unwrap(),
        ]))
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(found.get("status").unwrap().str().unwrap(), "UNAMBIGUOUS");
    let selected = found.get("selection").unwrap();
    assert_eq!(
        selected.get("scip_snapshot").unwrap().str().unwrap(),
        imported.get("snapshot_id").unwrap().str().unwrap()
    );
    assert_eq!(
        selected.get("source_prefix").unwrap().str().unwrap(),
        "packages/native"
    );
    let analyzed = json::parse(
        success(f.cli(&[
            "analyze",
            f.repo(),
            "--rev",
            &revision,
            "--db",
            database.to_str().unwrap(),
            "--json",
        ]))
        .as_bytes(),
    )
    .unwrap();
    let edges = analyzed.get("relationships").unwrap().array_ref().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0]
            .get("source")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap(),
        "packages/native/src/a.rs"
    );
}
