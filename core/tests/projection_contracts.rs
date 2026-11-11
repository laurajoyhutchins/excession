//! Native repository projection integration and adversarial replay tests.
//! The fixture is an immutable Git bundle; no mocked object identities.
use excession::json::{self, Value};
use excession::sqlite::Db;
use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().unwrap()
}
fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn denied(output: Output) {
    assert!(!output.status.success(), "unexpected acceptance");
}
fn exec(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(args: &[&str]) -> Value {
    json::parse(&success(exec(args))).unwrap()
}
fn raw(args: &[&str]) -> Vec<u8> {
    success(exec(args))
}
fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).unwrap().str().unwrap()
}
fn entries(v: &Value) -> &[Value] {
    v.get("entries").unwrap().array_ref().unwrap()
}
fn array<'a>(v: &'a Value, name: &str) -> &'a [Value] {
    v.get(name).unwrap().array_ref().unwrap()
}
fn save(path: &Path, v: &Value) {
    fs::write(path, v.canonical()).unwrap();
}
fn identities(v: &Value) -> BTreeSet<(String, String)> {
    entries(v)
        .iter()
        .map(|e| (field(e, "source_path").into(), field(e, "blob").into()))
        .collect()
}
fn paths(v: &Value) -> BTreeSet<String> {
    entries(v)
        .iter()
        .map(|e| field(e, "virtual_path").to_string())
        .collect()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
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
            "excession-projections-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repo = root.join("source");
        let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/h11-crossrepo.bundle");
        success(run(
            "git",
            &[
                "clone",
                "-q",
                bundle.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        ));
        let git = |spec: &str| {
            String::from_utf8(success(run(
                "git",
                &["-C", repo.to_str().unwrap(), "rev-parse", spec],
            )))
            .unwrap()
            .trim()
            .to_owned()
        };
        let commit = git("HEAD");
        let tree = git("HEAD^{tree}");
        let db = root.join("index.sqlite");
        raw(&[
            "index",
            "--db",
            db.to_str().unwrap(),
            "--name",
            "h11",
            "--repo",
            repo.to_str().unwrap(),
            "--rev",
            "HEAD",
            "--roots",
            "h11,tests",
        ]);
        Self {
            root,
            repo,
            db,
            commit,
            tree,
        }
    }
    fn repo(&self) -> &str {
        self.repo.to_str().unwrap()
    }
    fn db(&self) -> &str {
        self.db.to_str().unwrap()
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn plan(&self, seed: &str, name: &str) -> (Value, PathBuf) {
        let output = raw(&[
            "view",
            "deps",
            "--db",
            self.db(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            seed,
        ]);
        let file = self.path(name);
        fs::write(&file, &output).unwrap();
        (json::parse(&output).unwrap(), file)
    }
    fn security(&self, engine: &str) -> (Value, PathBuf, PathBuf, String) {
        let report = self.path(&format!("{engine}-report.json"));
        let db = self.path(&format!("{engine}-evidence.sqlite"));
        let text = if engine == "codeql" {
            r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"CodeQL"}},"results":[{"ruleId":"controlled/test-flow","locations":[{"physicalLocation":{"artifactLocation":{"uri":"h11/_connection.py"},"region":{"startLine":20}}}],"codeFlows":[{"threadFlows":[{"locations":[{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_connection.py"},"region":{"startLine":20}}}},{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_events.py"},"region":{"startLine":25}}}},{"location":{"physicalLocation":{"artifactLocation":{"uri":"h11/_util.py"},"region":{"startLine":20}}}}]}]}]}]}]}"#
        } else {
            r#"{"version":"1.0.0","errors":[],"results":[{"check_id":"controlled/review","path":"h11/_connection.py","start":{"line":20}}]}"#
        };
        fs::write(&report, text).unwrap();
        let imported = data(&[
            "security",
            "ingest",
            "--repo",
            self.repo(),
            "--report",
            report.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--engine",
            engine,
        ]);
        assert_eq!(field(&imported, "status"), "UNVERIFIED_STATIC_CANDIDATES");
        data(&[
            "security",
            "audit",
            "--repo",
            self.repo(),
            "--report",
            report.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
            "--engine",
            engine,
        ]);
        let run_id = field(&imported, "run_id").to_string();
        let planned = data(&[
            "view",
            "security",
            "--db",
            db.to_str().unwrap(),
            "--repo",
            self.repo(),
            "--run",
            &run_id,
        ]);
        let manifest = self.path(&format!("{engine}-view.json"));
        save(&manifest, &planned);
        (planned, manifest, db, run_id)
    }
    fn materialize(&self, manifest: &Path, output: &Path) -> Value {
        data(&[
            "view",
            "materialize",
            "--repo",
            self.repo(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
    }
}
#[test]
fn dependency_layout_is_deterministic_git_identical_and_readonly() {
    let f = Fixture::new();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    assert_eq!(
        raw(&[
            "view",
            "deps",
            "--db",
            f.db(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            "h11._connection",
        ]),
        fs::read(&manifest).unwrap()
    );
    assert_eq!(field(&plan, "schema"), "excession.repository-view.v1");
    assert_eq!(field(&plan, "source_commit"), f.commit);
    assert_eq!(field(&plan, "source_tree"), f.tree);
    assert!(
        entries(&plan)
            .iter()
            .any(|e| field(e, "role") == "indexed-test")
    );
    let vpaths = paths(&plan);
    assert_eq!(vpaths.len(), entries(&plan).len());
    let dest = f.path("read-only-view");
    let receipt = f.materialize(&manifest, &dest);
    assert_eq!(
        receipt.get("entries_verified").unwrap().integer().unwrap() as usize,
        entries(&plan).len()
    );
    assert_eq!(
        field(&receipt, "status"),
        "SOURCE_BLOBS_VERIFIED_NOT_EXECUTABLE"
    );
    for e in entries(&plan) {
        let file = dest.join(field(e, "virtual_path"));
        let source = format!("{}:{}", f.commit, field(e, "source_path"));
        assert_eq!(
            fs::read(&file).unwrap(),
            success(run("git", &["-C", f.repo(), "show", &source]))
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o444
        );
    }
    assert_eq!(
        data(&[
            "view",
            "audit",
            "--repo",
            f.repo(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--output",
            dest.to_str().unwrap(),
        ]),
        receipt
    );
    assert!(success(run("git", &["-C", f.repo(), "status", "--porcelain"])).is_empty());
}
#[test]
fn compact_paths_are_identical_to_full_view_by_role() {
    let f = Fixture::new();
    let (plan, _) = f.plan("h11._connection", "deps.json");
    for role in [
        "focus",
        "direct-dependencies",
        "transitive-dependencies",
        "direct-dependents",
        "transitive-dependents",
        "indexed-test",
    ] {
        let selected = data(&[
            "view",
            "paths",
            "--db",
            f.db(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            "h11._connection",
            "--role",
            role,
        ]);
        assert_eq!(field(&selected, "schema"), "excession.dependency-paths.v1");
        assert_eq!(field(&selected, "source_commit"), f.commit);
        assert_eq!(field(&selected, "source_tree"), f.tree);
        assert_eq!(
            selected.get("view_id").unwrap(),
            plan.get("view_id").unwrap()
        );
        assert_eq!(field(&selected, "role"), role);
        let expected = BTreeSet::from_iter(
            entries(&plan)
                .iter()
                .filter(|e| field(e, "role") == role)
                .map(|e| field(e, "source_path").to_string()),
        );
        let actual = BTreeSet::from_iter(
            array(&selected, "source_paths")
                .iter()
                .map(|v| v.str().unwrap().to_string()),
        );
        assert_eq!(actual, expected);
        assert!(
            selected.get("entries").is_err(),
            "compact path result is not a manifest"
        );
    }
    denied(exec(&[
        "view",
        "paths",
        "--db",
        f.db(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
        "--role",
        "unknown",
    ]));
}
#[test]
fn profile_uses_authenticated_source_and_reports_nonnegative_phases() {
    let f = Fixture::new();
    let (plan, _) = f.plan("h11._connection", "deps.json");
    let profile = data(&[
        "view",
        "profile-deps",
        "--db",
        f.db(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
    ]);
    assert_eq!(
        field(&profile, "schema"),
        "excession.projection-phase-profile.v1"
    );
    assert_eq!(field(&profile, "source_commit"), f.commit);
    assert_eq!(field(&profile, "source_tree"), f.tree);
    assert_eq!(
        profile.get("view_id").unwrap(),
        plan.get("view_id").unwrap()
    );
    assert_eq!(
        profile.get("entry_count").unwrap().integer().unwrap() as usize,
        entries(&plan).len()
    );
    let names = array(&profile, "phases")
        .iter()
        .map(|p| field(p, "phase"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "resolve_revision",
            "check_git_tree",
            "reconstruct_index_tree",
            "load_modules",
            "load_dependencies",
            "traverse_and_build_entries",
            "validate_index_and_plan",
            "validate_plan",
            "reuse_indexed_tree",
            "validate_source_bindings",
            "canonical_view_id"
        ]
    );
    let total: i64 = array(&profile, "phases")
        .iter()
        .map(|p| {
            let us = p.get("elapsed_us").unwrap().integer().unwrap();
            assert!(us >= 0);
            us
        })
        .sum();
    assert!(profile.get("total_us").unwrap().integer().unwrap() >= total);
    assert!(profile.get("entries").is_err());
}
#[test]
fn index_proof_rejects_tampered_module_and_tree_records() {
    let f = Fixture::new();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    for (name, sql) in [
        (
            "module",
            "UPDATE module SET blob_oid=(SELECT blob_oid FROM module WHERE name='h11._headers') WHERE name='h11._connection'",
        ),
        (
            "tree",
            "UPDATE tree_entry SET oid=(SELECT oid FROM tree_entry WHERE path='h11/_headers.py') WHERE path='h11/_connection.py'",
        ),
    ] {
        let altered = f.path(&format!("changed-{name}.sqlite"));
        fs::copy(&f.db, &altered).unwrap();
        let database = Db::open(&altered).unwrap();
        database.exec("PRAGMA foreign_keys=OFF").unwrap();
        database.exec(sql).unwrap();
        denied(exec(&[
            "view",
            "paths",
            "--db",
            altered.to_str().unwrap(),
            "--name",
            "h11",
            "--rev",
            &f.commit,
            "--seed",
            "h11._connection",
            "--role",
            "focus",
        ]));
    }
    let first = &entries(&plan)[0];
    let stat = data(&[
        "view",
        "stat",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--path",
        field(first, "virtual_path"),
    ]);
    assert_eq!(
        stat.get("entry").unwrap().get("blob").unwrap(),
        first.get("blob").unwrap()
    );
}
#[test]
fn composition_checks_both_inputs_and_enumerates_git_once() {
    let f = Fixture::new();
    let (_, a) = f.plan("h11._connection", "deps.json");
    let (_, b) = f.plan("h11._headers", "headers.json");
    let left = f.path("left.json");
    let right = f.path("right.json");
    fs::write(
        &left,
        raw(&[
            "view",
            "mount",
            "--repo",
            f.repo(),
            "--manifest",
            a.to_str().unwrap(),
            "--namespace",
            "left",
        ]),
    )
    .unwrap();
    fs::write(
        &right,
        raw(&[
            "view",
            "mount",
            "--repo",
            f.repo(),
            "--manifest",
            b.to_str().unwrap(),
            "--namespace",
            "right",
        ]),
    )
    .unwrap();
    let args = [
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        left.to_str().unwrap(),
        "--right",
        right.to_str().unwrap(),
    ];
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(args)
        .env("EXCESSION_TIMING", "1")
        .output()
        .unwrap();
    success(output.clone());
    let timing = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        timing
            .lines()
            .filter(|line| line.contains(" ls-tree "))
            .count(),
        1,
        "composition should share the enumeration: {timing}"
    );
    let mut forged = json::parse(&fs::read(&right).unwrap()).unwrap();
    let mut list = entries(&forged).to_vec();
    list[0]
        .entries_mut()
        .unwrap()
        .insert("blob".into(), Value::string("0".repeat(40)));
    forged
        .entries_mut()
        .unwrap()
        .insert("entries".into(), Value::array(list));
    reseal(&mut forged);
    let bad = f.path("forged-right.json");
    save(&bad, &forged);
    let forged_args = [
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        left.to_str().unwrap(),
        "--right",
        bad.to_str().unwrap(),
    ];
    let refusal = exec(&forged_args);
    denied(refusal.clone());
    assert!(String::from_utf8_lossy(&refusal.stderr).contains("view source identity mismatch"));
}
fn reseal(report: &mut Value) {
    report.entries_mut().unwrap().remove("view_id").unwrap();
    let digest = report.sha256();
    report
        .entries_mut()
        .unwrap()
        .insert("view_id".into(), Value::string(digest));
}
#[test]
fn native_graph_baseline_matches_authenticated_view() {
    let f = Fixture::new();
    let (plan, _) = f.plan("h11._connection", "deps.json");
    for (kind, roles) in [
        ("focus", vec!["focus"]),
        ("dependencies_direct", vec!["direct-dependencies"]),
        (
            "dependencies_all",
            vec!["direct-dependencies", "transitive-dependencies"],
        ),
        ("dependents_direct", vec!["direct-dependents"]),
        (
            "dependents_all",
            vec!["direct-dependents", "transitive-dependents"],
        ),
        ("tests", vec!["indexed-test"]),
    ] {
        let graph = data(&[
            "view",
            "graph",
            "--db",
            f.db(),
            "--name",
            "h11",
            "--rev",
            "HEAD",
            "--seed",
            "h11._connection",
            "--kind",
            kind,
        ]);
        assert_eq!(field(&graph, "schema"), "excession.graph-baseline.v1");
        assert_eq!(field(&graph, "source_commit"), f.commit);
        assert_eq!(field(&graph, "source_tree"), f.tree);
        assert_eq!(field(&graph, "kind"), kind);
        let expect = BTreeSet::from_iter(
            entries(&plan)
                .iter()
                .filter(|v| roles.contains(&field(v, "role")))
                .map(|v| field(v, "source_path").to_string()),
        );
        let got = BTreeSet::from_iter(
            array(&graph, "source_paths")
                .iter()
                .map(|x| x.str().unwrap().to_string()),
        );
        assert_eq!(got, expect);
    }
    denied(exec(&[
        "view",
        "graph",
        "--db",
        f.db(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
        "--kind",
        "bad-kind",
    ]));
}
#[test]
fn changing_focus_changes_layout_not_pinned_revision() {
    let f = Fixture::new();
    let (a, _) = f.plan("h11._connection", "a.json");
    let (b, _) = f.plan("h11._headers", "b.json");
    assert_ne!(a.get("view_id").unwrap(), b.get("view_id").unwrap());
    assert_eq!(
        a.get("source_commit").unwrap(),
        b.get("source_commit").unwrap()
    );
    assert_ne!(paths(&a), paths(&b));
}
#[test]
fn replay_refuses_missing_index_relations() {
    let f = Fixture::new();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    let receipt = data(&[
        "view",
        "replay",
        "deps",
        "--db",
        f.db(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(
        receipt.get("view_id").unwrap(),
        plan.get("view_id").unwrap()
    );
    let db = Db::open(&f.db).unwrap();
    assert!(
        !db.query(
            "SELECT source_name FROM dependency WHERE source_name='h11._connection'",
            &[]
        )
        .unwrap()
        .is_empty()
    );
    db.exec("DELETE FROM dependency WHERE source_name='h11._connection'")
        .unwrap();
    denied(exec(&[
        "view",
        "replay",
        "deps",
        "--db",
        f.db(),
        "--name",
        "h11",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
        "--manifest",
        manifest.to_str().unwrap(),
    ]));
}
#[test]
fn codeql_flow_layout_and_semgrep_candidate_are_conservative() {
    let f = Fixture::new();
    let (flow, manifest, db, run_id) = f.security("codeql");
    assert_eq!(field(&flow, "kind"), "security-flow");
    assert_eq!(entries(&flow).len(), 4);
    assert_eq!(field(&flow, "source_commit"), f.commit);
    let roles = BTreeSet::from_iter(entries(&flow).iter().map(|e| field(e, "role")));
    assert_eq!(
        roles,
        BTreeSet::from([
            "unverified-static-candidate",
            "scanner-flow-start",
            "scanner-flow-intermediate",
            "scanner-flow-end"
        ])
    );
    let target = f.path("security-tree");
    let receipt = f.materialize(&manifest, &target);
    assert_eq!(
        receipt.get("entries_verified").unwrap().integer().unwrap(),
        4
    );
    data(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        target.to_str().unwrap(),
    ]);
    let replay = data(&[
        "view",
        "replay",
        "security",
        "--db",
        db.to_str().unwrap(),
        "--repo",
        f.repo(),
        "--run",
        &run_id,
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(replay.get("view_id").unwrap(), flow.get("view_id").unwrap());
    let (static_view, manifest, _, _) = f.security("semgrep");
    assert_eq!(entries(&static_view).len(), 1);
    assert_eq!(
        field(&entries(&static_view)[0], "role"),
        "unverified-static-candidate"
    );
    f.materialize(&manifest, &f.path("semgrep-view"));
}
#[test]
fn audit_refuses_missing_modified_extra_and_symlink_files() {
    let f = Fixture::new();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    let dest = f.path("view");
    f.materialize(&manifest, &dest);
    let first = dest.join(field(&entries(&plan)[0], "virtual_path"));
    #[cfg(unix)]
    fs::set_permissions(&first, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&first, "tampered\n").unwrap();
    denied(exec(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]));
    fs::remove_file(&first).unwrap();
    denied(exec(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]));

    let extra = f.path("fresh-view");
    f.materialize(&manifest, &extra);
    fs::write(extra.join("views/untracked.txt"), "not in manifest").unwrap();
    denied(exec(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        extra.to_str().unwrap(),
    ]));
    #[cfg(unix)]
    {
        fs::remove_file(extra.join("views/untracked.txt")).unwrap();
        let target = extra.join(field(&entries(&plan)[0], "virtual_path"));
        fs::remove_file(&target).unwrap();
        symlink(
            f.repo.join(field(&entries(&plan)[0], "source_path")),
            &target,
        )
        .unwrap();
        denied(exec(&[
            "view",
            "audit",
            "--repo",
            f.repo(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--output",
            extra.to_str().unwrap(),
        ]));
    }
}
#[test]
fn manifest_tamper_and_unsafe_materialization_are_refused() {
    let f = Fixture::new();
    let (mut plan, manifest) = f.plan("h11._connection", "deps.json");
    let dest = f.path("view");
    f.materialize(&manifest, &dest);
    let mut list = entries(&plan).to_vec();
    list[0]
        .entries_mut()
        .unwrap()
        .insert("source_path".into(), Value::string("h11/_abnf.py"));
    plan.entries_mut()
        .unwrap()
        .insert("entries".into(), Value::array(list));
    let changed = f.path("forged.json");
    save(&changed, &plan);
    denied(exec(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        changed.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]));
    denied(exec(&[
        "view",
        "materialize",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]));
    let inside = f.repo.join("new-view");
    denied(exec(&[
        "view",
        "materialize",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        inside.to_str().unwrap(),
    ]));
    assert!(!inside.exists());
    denied(exec(&[
        "view",
        "materialize",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        "relative-view",
    ]));
}
#[test]
fn security_plan_replay_refuses_deleted_evidence_steps() {
    let f = Fixture::new();
    let (_, manifest, db, run_id) = f.security("codeql");
    Db::open(&db)
        .unwrap()
        .execute(
            "DELETE FROM security_ingest_flow_step WHERE run_id=? AND step_index=1",
            &[excession::sqlite::Param::Text(&run_id)],
        )
        .unwrap();
    denied(exec(&[
        "view",
        "replay",
        "security",
        "--db",
        db.to_str().unwrap(),
        "--repo",
        f.repo(),
        "--run",
        &run_id,
        "--manifest",
        manifest.to_str().unwrap(),
    ]));
}
#[test]
fn pinned_git_materialization_ignores_mutable_worktree() {
    let f = Fixture::new();
    let (_, manifest) = f.plan("h11._connection", "deps.json");
    let stray = f.repo.join("untracked-temporary.txt");
    fs::write(&stray, "not in Git").unwrap();
    let dest = f.path("pinned-view");
    f.materialize(&manifest, &dest);
    data(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]);
}
#[test]
fn virtual_git_navigation_reads_exact_blobs_without_copying() {
    let f = Fixture::new();
    let (plan, manifest) = f.plan("h11._connection", "deps.json");
    let list = data(&[
        "view",
        "ls",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(array(&list, "children").len(), 1);
    assert_eq!(field(&array(&list, "children")[0], "name"), "views");
    let groups = data(&[
        "view",
        "ls",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--path",
        "views",
    ]);
    assert!(
        array(&groups, "children")
            .iter()
            .any(|v| field(v, "name") == "00-focus")
    );
    let entry = entries(&plan)
        .iter()
        .find(|v| field(v, "role") == "focus")
        .unwrap();
    let vpath = field(entry, "virtual_path");
    let parent = Path::new(vpath).parent().unwrap().to_str().unwrap();
    let children = data(&[
        "view",
        "ls",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--path",
        parent,
    ]);
    assert_eq!(array(&children, "children").len(), 1);
    let desc = data(&[
        "view",
        "stat",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--path",
        vpath,
    ]);
    assert_eq!(
        desc.get("entry").unwrap().get("blob").unwrap(),
        entry.get("blob").unwrap()
    );
    assert_eq!(
        desc.get("entry").unwrap().get("source_path").unwrap(),
        entry.get("source_path").unwrap()
    );
    let bytes = raw(&[
        "view",
        "cat",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--path",
        vpath,
    ]);
    let spec = format!("{}:{}", f.commit, field(entry, "source_path"));
    assert_eq!(bytes, success(run("git", &["-C", f.repo(), "show", &spec])));
}
#[test]
fn compositions_and_selections_are_replayable() {
    let f = Fixture::new();
    let (deps, dep) = f.plan("h11._connection", "deps.json");
    let (security, sec, _, _) = f.security("codeql");
    let args = [
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        dep.to_str().unwrap(),
        "--right",
        sec.to_str().unwrap(),
    ];
    let bytes = raw(&args);
    assert_eq!(bytes, raw(&args));
    let joined = json::parse(&bytes).unwrap();
    assert_eq!(field(&joined, "kind"), "composed");
    assert_eq!(field(&joined, "source_commit"), f.commit);
    assert_eq!(
        entries(&joined).len(),
        entries(&deps).len() + entries(&security).len()
    );
    let output = f.path("union.json");
    fs::write(&output, &bytes).unwrap();
    let replay = data(&[
        "view",
        "replay",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        dep.to_str().unwrap(),
        "--right",
        sec.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(
        replay.get("view_id").unwrap(),
        joined.get("view_id").unwrap()
    );
    let selected = data(&[
        "view",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        output.to_str().unwrap(),
        "--axis",
        "role",
        "--value",
        "scanner-flow-start",
    ]);
    assert_eq!(entries(&selected).len(), 1);
    assert_eq!(field(&entries(&selected)[0], "role"), "scanner-flow-start");
    let filtered = f.path("filtered.json");
    save(&filtered, &selected);
    data(&[
        "view",
        "replay",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        output.to_str().unwrap(),
        "--axis",
        "role",
        "--value",
        "scanner-flow-start",
        "--output",
        filtered.to_str().unwrap(),
    ]);
    let dest = f.path("union-view");
    f.materialize(&output, &dest);
    data(&[
        "view",
        "audit",
        "--repo",
        f.repo(),
        "--manifest",
        output.to_str().unwrap(),
        "--output",
        dest.to_str().unwrap(),
    ]);
}
#[test]
fn intersection_difference_and_source_filter_preserve_identity() {
    let f = Fixture::new();
    let (deps, manifest) = f.plan("h11._connection", "deps.json");
    let focus = data(&[
        "view",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--axis",
        "role",
        "--value",
        "focus",
    ]);
    let chosen = f.path("focus.json");
    save(&chosen, &focus);
    let intersection = data(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "intersection",
        "--left",
        manifest.to_str().unwrap(),
        "--right",
        chosen.to_str().unwrap(),
    ]);
    assert_eq!(identities(&intersection), identities(&focus));
    let difference = data(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "difference",
        "--left",
        manifest.to_str().unwrap(),
        "--right",
        chosen.to_str().unwrap(),
    ]);
    assert!(identities(&difference).is_disjoint(&identities(&focus)));
    let both = paths(&intersection)
        .union(&paths(&difference))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(paths(&deps), both);
    let narrowed = data(&[
        "view",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--axis",
        "source-prefix",
        "--value",
        "h11",
    ]);
    assert!(!entries(&narrowed).is_empty() && entries(&narrowed).len() < entries(&deps).len());
    assert!(
        entries(&narrowed)
            .iter()
            .all(|e| field(e, "source_path").starts_with("h11/"))
    );
    let virtual_focus = data(&[
        "view",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--axis",
        "virtual-prefix",
        "--value",
        "views/00-focus",
    ]);
    assert_eq!(entries(&virtual_focus).len(), 1);
}
#[test]
fn replay_refuses_changed_inputs_and_output() {
    let f = Fixture::new();
    let (_, dep) = f.plan("h11._connection", "deps.json");
    let (_, sec, _, _) = f.security("codeql");
    let result = data(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        dep.to_str().unwrap(),
        "--right",
        sec.to_str().unwrap(),
    ]);
    let output = f.path("combined.json");
    save(&output, &result);
    denied(exec(&[
        "view",
        "replay",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "difference",
        "--left",
        dep.to_str().unwrap(),
        "--right",
        sec.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]));
    let mut altered = json::parse(&fs::read(&dep).unwrap()).unwrap();
    let mut list = entries(&altered).to_vec();
    list[0]
        .entries_mut()
        .unwrap()
        .insert("role".into(), Value::string("fabricated-role"));
    altered
        .entries_mut()
        .unwrap()
        .insert("entries".into(), Value::array(list));
    reseal(&mut altered);
    let bad = f.path("forged.json");
    save(&bad, &altered);
    denied(exec(&[
        "view",
        "replay",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        bad.to_str().unwrap(),
        "--right",
        sec.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]));
    denied(exec(&[
        "view",
        "replay",
        "select",
        "--repo",
        f.repo(),
        "--manifest",
        dep.to_str().unwrap(),
        "--axis",
        "role",
        "--value",
        "focus",
        "--output",
        output.to_str().unwrap(),
    ]));
}
#[test]
fn virtual_operations_refuse_invalid_scope_or_empty_composition() {
    let f = Fixture::new();
    let (_, dep) = f.plan("h11._connection", "deps.json");
    let root = f.repo();
    let manifest = dep.to_str().unwrap();
    for args in [
        vec![
            "view",
            "compose",
            "--repo",
            root,
            "--op",
            "xor",
            "--left",
            manifest,
            "--right",
            manifest,
        ],
        vec![
            "view",
            "select",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--axis",
            "role",
            "--value",
            "does-not-exist",
        ],
        vec![
            "view",
            "select",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--axis",
            "source-prefix",
            "--value",
            "../bad",
        ],
        vec![
            "view",
            "ls",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--path",
            "views/../bad",
        ],
        vec![
            "view",
            "ls",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--path",
            "views/nonexistent",
        ],
        vec![
            "view",
            "cat",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--path",
            "views/../../etc/passwd",
        ],
        vec![
            "view",
            "stat",
            "--repo",
            root,
            "--manifest",
            manifest,
            "--path",
            "views/not-a-file",
        ],
        vec![
            "view",
            "compose",
            "--repo",
            root,
            "--op",
            "difference",
            "--left",
            manifest,
            "--right",
            manifest,
        ],
    ] {
        denied(exec(&args));
    }
    let focus = data(&[
        "view",
        "select",
        "--repo",
        root,
        "--manifest",
        manifest,
        "--axis",
        "role",
        "--value",
        "focus",
    ]);
    let tests = data(&[
        "view",
        "select",
        "--repo",
        root,
        "--manifest",
        manifest,
        "--axis",
        "role",
        "--value",
        "indexed-test",
    ]);
    let a = f.path("focus.json");
    let b = f.path("tests.json");
    save(&a, &focus);
    save(&b, &tests);
    denied(exec(&[
        "view",
        "compose",
        "--repo",
        root,
        "--op",
        "intersection",
        "--left",
        a.to_str().unwrap(),
        "--right",
        b.to_str().unwrap(),
    ]));
}
#[test]
fn composition_refuses_cross_revision_views() {
    let f = Fixture::new();
    let (_, first) = f.plan("h11._connection", "deps.json");
    let alternate = f.path("different-revision");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/h11-crossrepo.bundle");
    success(run(
        "git",
        &[
            "clone",
            "-q",
            bundle.to_str().unwrap(),
            alternate.to_str().unwrap(),
        ],
    ));
    let modified = alternate.join("h11/_util.py");
    let mut bytes = fs::read(&modified).unwrap();
    bytes.extend_from_slice(b"\n# alternate revision\n");
    fs::write(&modified, bytes).unwrap();
    let root = alternate.to_str().unwrap();
    success(run("git", &["-C", root, "add", "h11/_util.py"]));
    success(run(
        "git",
        &[
            "-C",
            root,
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "alternate source",
        ],
    ));
    let db = f.path("alternate.sqlite");
    raw(&[
        "index",
        "--db",
        db.to_str().unwrap(),
        "--name",
        "alternate",
        "--repo",
        root,
        "--rev",
        "HEAD",
        "--roots",
        "h11,tests",
    ]);
    let bytes = raw(&[
        "view",
        "deps",
        "--db",
        db.to_str().unwrap(),
        "--name",
        "alternate",
        "--rev",
        "HEAD",
        "--seed",
        "h11._connection",
    ]);
    let other = f.path("different.json");
    fs::write(&other, bytes).unwrap();
    denied(exec(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        first.to_str().unwrap(),
        "--right",
        other.to_str().unwrap(),
    ]));
    denied(exec(&[
        "view",
        "compose",
        "--repo",
        root,
        "--op",
        "union",
        "--left",
        first.to_str().unwrap(),
        "--right",
        other.to_str().unwrap(),
    ]));
}
#[test]
fn union_conflicts_and_file_directory_collisions_are_refused() {
    let f = Fixture::new();
    let (_, manifest) = f.plan("h11._connection", "deps.json");
    let mut changed = json::parse(&fs::read(&manifest).unwrap()).unwrap();
    let mut records = entries(&changed).to_vec();
    records[0]
        .entries_mut()
        .unwrap()
        .insert("role".into(), Value::string("different-role"));
    changed
        .entries_mut()
        .unwrap()
        .insert("entries".into(), Value::array(records));
    reseal(&mut changed);
    let other = f.path("changed-role.json");
    save(&other, &changed);
    denied(exec(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        manifest.to_str().unwrap(),
        "--right",
        other.to_str().unwrap(),
    ]));
    let mut malformed = json::parse(&fs::read(&manifest).unwrap()).unwrap();
    let mut records = entries(&malformed).to_vec();
    let mut extra = records[0].clone();
    extra
        .entries_mut()
        .unwrap()
        .insert("virtual_path".into(), Value::string("views/00-focus"));
    records.push(extra);
    records.sort_by(|a, b| field(a, "virtual_path").cmp(field(b, "virtual_path")));
    malformed
        .entries_mut()
        .unwrap()
        .insert("entries".into(), Value::array(records));
    reseal(&mut malformed);
    let wrong = f.path("file-dir.json");
    save(&wrong, &malformed);
    denied(exec(&[
        "view",
        "ls",
        "--repo",
        f.repo(),
        "--manifest",
        wrong.to_str().unwrap(),
        "--path",
        "views",
    ]));
}
#[test]
fn namespace_mounts_are_disjoint_and_replayable() {
    let f = Fixture::new();
    let (a, a_path) = f.plan("h11._connection", "a.json");
    let (b, b_path) = f.plan("h11._headers", "b.json");
    let mounted_a = data(&[
        "view",
        "mount",
        "--repo",
        f.repo(),
        "--manifest",
        a_path.to_str().unwrap(),
        "--namespace",
        "connection",
    ]);
    let mounted_b = data(&[
        "view",
        "mount",
        "--repo",
        f.repo(),
        "--manifest",
        b_path.to_str().unwrap(),
        "--namespace",
        "headers",
    ]);
    assert_eq!(entries(&mounted_a).len(), entries(&a).len());
    assert_eq!(entries(&mounted_b).len(), entries(&b).len());
    assert!(
        entries(&mounted_a)
            .iter()
            .all(|e| field(e, "virtual_path").starts_with("views/connection/"))
    );
    assert!(
        entries(&mounted_b)
            .iter()
            .all(|e| field(e, "virtual_path").starts_with("views/headers/"))
    );
    let left = f.path("mount-a.json");
    let right = f.path("mount-b.json");
    save(&left, &mounted_a);
    save(&right, &mounted_b);
    let union = data(&[
        "view",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        left.to_str().unwrap(),
        "--right",
        right.to_str().unwrap(),
    ]);
    assert_eq!(entries(&union).len(), entries(&a).len() + entries(&b).len());
    let both = f.path("both.json");
    save(&both, &union);
    data(&[
        "view",
        "replay",
        "compose",
        "--repo",
        f.repo(),
        "--op",
        "union",
        "--left",
        left.to_str().unwrap(),
        "--right",
        right.to_str().unwrap(),
        "--output",
        both.to_str().unwrap(),
    ]);
    data(&[
        "view",
        "replay",
        "mount",
        "--repo",
        f.repo(),
        "--manifest",
        a_path.to_str().unwrap(),
        "--namespace",
        "connection",
        "--output",
        left.to_str().unwrap(),
    ]);
    let branch = data(&[
        "view",
        "ls",
        "--repo",
        f.repo(),
        "--manifest",
        both.to_str().unwrap(),
        "--path",
        "views",
    ]);
    let got = BTreeSet::from_iter(array(&branch, "children").iter().map(|v| field(v, "name")));
    assert_eq!(got, BTreeSet::from(["connection", "headers"]));
    for name in ["../bad", "slash/not-allowed", "", "control\nchar"] {
        denied(exec(&[
            "view",
            "mount",
            "--repo",
            f.repo(),
            "--manifest",
            a_path.to_str().unwrap(),
            "--namespace",
            name,
        ]));
    }
}
