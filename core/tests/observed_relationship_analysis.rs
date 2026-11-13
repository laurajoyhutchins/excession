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
        let store = Db::open(&self.db).unwrap();
        let blob = git::blob(&self.repo, &self.head, "src/b.rs").unwrap();
        let tree = git::rev(&self.repo, &format!("{}^{{tree}}", self.head)).unwrap();
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        configured_store::push_node(
            &mut nodes,
            "target:demo".into(),
            "configured-target",
            obj(&[("name", Value::string("demo"))]),
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
            "target:demo",
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
            ("target", Value::string("demo")),
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
            target: "demo",
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
    // Without an explicitly selected index, the ordinary module analyzer
    // still refuses unsupported Rust-only semantic analysis.
    refuse(f.cli(&["analyze", f.repo(), "--json"]));
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
