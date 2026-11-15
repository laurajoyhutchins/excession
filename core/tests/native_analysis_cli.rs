//! A single contract across native Go input discovery and Clang AST receipts.
//! Requires no Tree-sitter installation; only the selected native toolchain.
use excession::json::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    parent: PathBuf,
    repo: PathBuf,
    clang_db: PathBuf,
    commit: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}
fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}
fn ok(program: &str, args: &[&str], dir: &Path) -> Vec<u8> {
    let output = run(program, args, dir);
    assert!(
        output.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn available(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .output()
        .is_ok_and(|v| v.status.success())
}
impl Fixture {
    fn new() -> Self {
        let parent = std::env::temp_dir().join(format!(
            "excession-native-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = parent.join("checkout");
        fs::create_dir_all(&repo).unwrap();
        ok("git", &["init", "-q"], &repo);
        for (path, source) in [
            ("go.mod", "module example.org/mini\n\ngo 1.20\n"),
            (
                "cmd/server/main.go",
                "package server\nimport \"example.org/mini/dep\"\nfunc Answer() int { return dep.Value }\n",
            ),
            ("dep/dep.go", "package dep\nconst Value = 42\n"),
            (
                "src/main.c",
                "#include \"main.h\"\nint entry(int a) { return helper(a); }\n",
            ),
            (
                "src/main.h",
                "static inline int helper(int n) { return n + 1; }\n",
            ),
            (
                "src/worker.cpp",
                "namespace app { int twice(int n) { return n+n; } }\n",
            ),
        ] {
            let file = repo.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, source).unwrap();
        }
        ok("git", &["add", "."], &repo);
        ok(
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "source",
            ],
            &repo,
        );
        let commit = String::from_utf8(ok("git", &["rev-parse", "HEAD"], &repo))
            .unwrap()
            .trim()
            .to_string();
        let clang_db = parent.join("compile_commands.json");
        let cwd = repo.display().to_string();
        let db = Value::array([
            excession::json::obj(&[
                ("directory", Value::string(&cwd)),
                ("file", Value::string("src/main.c")),
                (
                    "arguments",
                    Value::array([
                        Value::string("clang"),
                        Value::string("-c"),
                        Value::string("src/main.c"),
                    ]),
                ),
            ]),
            excession::json::obj(&[
                ("directory", Value::string(&cwd)),
                ("file", Value::string("src/worker.cpp")),
                (
                    "arguments",
                    Value::array([
                        Value::string("clang++"),
                        Value::string("-std=c++17"),
                        Value::string("-c"),
                        Value::string("src/worker.cpp"),
                    ]),
                ),
            ]),
        ]);
        fs::write(&clang_db, db.canonical()).unwrap();
        Self {
            parent,
            repo,
            clang_db,
            commit,
        }
    }
    fn invoke(&self, args: &[&str]) -> Output {
        run(env!("CARGO_BIN_EXE_excession"), args, &self.repo)
    }
    fn db(&self) -> &str {
        self.clang_db.to_str().unwrap()
    }
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).unwrap().str().unwrap()
}
fn parsed(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    json::parse(&output.stdout).unwrap()
}
#[test]
fn native_go_inputs_have_git_blobs_and_no_fabricated_call_graph() {
    if !available("go", "version") || !available("python3", "--version") {
        return;
    }
    let f = Fixture::new();
    let response = parsed(f.invoke(&[
        "analyze",
        "native",
        "--language",
        "go",
        "--source",
        "cmd/server/main.go",
        "--trusted-local",
        "--json",
    ]));
    assert_eq!(field(&response, "schema"), "excession.native-analysis.v1");
    assert_eq!(field(&response, "scope"), "COMPILER_SELECTED_INPUTS_ONLY");
    assert_eq!(field(&response, "source_commit"), f.commit);
    assert!(response.content_id_matches("report_id").unwrap());
    assert_eq!(
        field(response.get("source").unwrap(), "path"),
        "cmd/server/main.go"
    );
    let inputs = response.get("inputs").unwrap().array_ref().unwrap();
    let paths = inputs
        .iter()
        .map(|e| field(e.get("source").unwrap(), "path"))
        .collect::<Vec<_>>();
    for expected in ["cmd/server/main.go", "dep/dep.go", "go.mod"] {
        assert!(
            paths.contains(&expected),
            "{expected} not found in {paths:?}"
        );
    }
    for record in inputs {
        let item = record.get("source").unwrap();
        let path = field(item, "path");
        let expected =
            String::from_utf8(ok("git", &["rev-parse", &format!("HEAD:{path}")], &f.repo)).unwrap();
        assert_eq!(field(item, "blob"), expected.trim());
        assert_eq!(field(item, "source_tree"), field(&response, "source_tree"));
    }
    assert_eq!(field(&response, "execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
    assert!(response.get("observation").unwrap().get("imports").is_err());
    let human = f.invoke(&[
        "analyze",
        "native",
        "--language",
        "go",
        "--source",
        "cmd/server/main.go",
        "--trusted-local",
    ]);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(String::from_utf8_lossy(&human.stdout).contains("COMPILER_SELECTED_INPUTS_ONLY"));
}

#[test]
fn native_c_and_cpp_preserve_configured_ast_and_unverified_header_scope() {
    if !available("clang", "--version") || !available("clang++", "--version") {
        return;
    }
    let f = Fixture::new();
    for (language, source, expected) in [
        ("c", "src/main.c", "entry"),
        ("cpp", "src/worker.cpp", "twice"),
    ] {
        let response = parsed(f.invoke(&[
            "analyze",
            "native",
            "--language",
            language,
            "--source",
            source,
            "--compile-db",
            f.db(),
            "--trusted-local",
            "--json",
        ]));
        assert!(response.content_id_matches("report_id").unwrap());
        assert_eq!(
            field(&response, "scope"),
            "CONFIGURED_TU_NOT_BUILD_VERIFIED"
        );
        assert_eq!(field(&response, "source_commit"), f.commit);
        let native = response
            .get("observation")
            .unwrap()
            .get("observation")
            .unwrap();
        let symbols = native.get("symbols").unwrap().array_ref().unwrap();
        assert!(
            symbols.iter().any(|s| field(s, "name") == expected),
            "{source}: {symbols:?}"
        );
        assert_eq!(field(&response, "execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
        assert!(
            response
                .get("limitations")
                .unwrap()
                .array_ref()
                .unwrap()
                .iter()
                .any(|l| l.str().unwrap().contains("not fully Git-authenticated"))
        );
    }
}

#[test]
fn native_refuses_unsupported_languages_dirty_worktrees_and_wrong_revision() {
    let f = Fixture::new();
    for extra in [
        vec![
            "--language",
            "java",
            "--source",
            "src/Main.java",
            "--trusted-local",
        ],
        vec![
            "--language",
            "cpp",
            "--source",
            "src/worker.cpp",
            "--trusted-local",
        ],
        vec!["--language", "go", "--source", "cmd/server/main.go"],
    ] {
        let mut args = vec!["analyze", "native"];
        args.extend(extra);
        let out = f.invoke(&args);
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }
    fs::write(f.repo.join("src/main.c"), "int changed = 1;\n").unwrap();
    let out = f.invoke(&[
        "analyze",
        "native",
        "--language",
        "c",
        "--source",
        "src/main.c",
        "--compile-db",
        f.db(),
        "--trusted-local",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("dirty checkout"));
}

#[test]
fn native_go_receipts_replay_into_analysis_impact_and_agent_without_fabricated_edges() {
    if !available("go", "version") || !available("python3", "--version") {
        return;
    }
    let f = Fixture::new();
    let database = f.parent.join("evidence.sqlite");
    let db = database.to_str().unwrap();
    let native = parsed(f.invoke(&[
        "analyze",
        "native",
        "--language",
        "go",
        "--source",
        "cmd/server/main.go",
        "--trusted-local",
        "--json",
    ]));
    let report = f.parent.join("native-go.json");
    fs::write(&report, native.canonical()).unwrap();
    let report_path = report.to_str().unwrap();
    let import_args = [
        "evidence",
        "native-import",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--report",
        report_path,
        "--db",
        db,
    ];
    let ingested = parsed(f.invoke(&import_args));
    assert_eq!(
        field(&ingested, "schema"),
        "excession.native-evidence-import.v1"
    );
    assert_eq!(
        field(&ingested, "semantics"),
        "PRODUCER_SUPPLIED_UNATTESTED"
    );
    let snapshot = field(&ingested, "snapshot_id");
    let repeated = parsed(f.invoke(&import_args));
    assert_eq!(
        snapshot,
        field(&repeated, "snapshot_id"),
        "import must be idempotent"
    );
    let replay = parsed(f.invoke(&[
        "evidence",
        "native-store-audit",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--db",
        db,
        "--snapshot",
        snapshot,
    ]));
    assert_eq!(
        field(&replay, "status"),
        "REPLAYED_GIT_BOUND_NATIVE_OBSERVATIONS"
    );
    assert_eq!(field(&replay, "producer_attestation"), "NOT_ESTABLISHED");

    let analyzed = parsed(f.invoke(&["analyze", ".", "--db", db, "--json"]));
    assert_eq!(field(&analyzed, "source_commit"), f.commit);
    let selection = analyzed
        .get("producer_selection_plan")
        .unwrap()
        .get("native")
        .unwrap()
        .array_ref()
        .unwrap();
    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].str().unwrap(), snapshot);
    let configured = analyzed
        .get("configured_observations")
        .unwrap()
        .array_ref()
        .unwrap();
    assert!(
        configured.iter().any(
            |item| field(item.get("source").unwrap(), "path") == "dep/dep.go"
                && field(item, "native_kind") == "NATIVE_SELECTED_INPUT"
        )
    );
    assert!(
        analyzed
            .get("relationships")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .all(|item| field(item, "kind") != "GO_DEPENDENCY")
    );
    let context = parsed(f.invoke(&[
        "agent",
        "context",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--focus",
        "dep/dep.go",
        "--db",
        db,
    ]));
    assert!(
        context
            .get("configured_observation_count")
            .unwrap()
            .integer()
            .unwrap()
            > 0
    );

    fs::write(f.repo.join("dep/dep.go"), "package dep\nconst Value = 43\n").unwrap();
    ok("git", &["add", "dep/dep.go"], &f.repo);
    ok(
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "change Go dependency",
        ],
        &f.repo,
    );
    let head = String::from_utf8(ok("git", &["rev-parse", "HEAD"], &f.repo))
        .unwrap()
        .trim()
        .to_owned();
    let new_report = parsed(f.invoke(&["analyze", ".", "--db", db, "--json"]));
    assert!(
        new_report
            .get("producer_selection_plan")
            .unwrap()
            .entries()
            .unwrap()
            .get("native")
            .is_none(),
        "old native receipts must not apply to a different commit"
    );
    let impact = parsed(f.invoke(&[
        "impact", ".", "--db", db, "--base", &f.commit, "--head", &head, "--json",
    ]));
    assert!(
        impact
            .get("configured_observations")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .any(|item| field(item.get("source").unwrap(), "path") == "dep/dep.go")
    );
    let verified = parsed(f.invoke(&[
        "agent",
        "verify",
        "--repo",
        f.repo.to_str().unwrap(),
        "--base",
        &f.commit,
        "--candidate",
        &head,
        "--db",
        db,
    ]));
    assert!(!verified.get("ready_to_merge").unwrap().bool().unwrap());
    assert!(
        verified
            .get("configured_observation_count")
            .unwrap()
            .integer()
            .unwrap()
            > 0
    );
    assert!(
        verified
            .get("obligations")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .any(|item| item.str().unwrap() == "NATIVE_PRODUCER_ATTESTATION_MISSING")
    );
    let stale = f.invoke(&[
        "evidence",
        "native-store-audit",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &head,
        "--snapshot",
        snapshot,
        "--db",
        db,
    ]);
    assert!(!stale.status.success(), "cross-revision replay must fail");

    // Recalculate the outer report ID after forging a Git blob. A valid
    // content hash must not bypass independent Git source authentication.
    let mut forged = native.clone();
    let first = match forged.entries_mut().unwrap().get_mut("inputs").unwrap() {
        Value::Array(items) => items.first_mut().unwrap(),
        _ => panic!("inputs must be an array"),
    };
    first
        .entries_mut()
        .unwrap()
        .get_mut("source")
        .unwrap()
        .entries_mut()
        .unwrap()
        .insert(
            "blob".to_owned(),
            Value::string("0000000000000000000000000000000000000000"),
        );
    forged.entries_mut().unwrap().remove("report_id");
    let forged = forged.stamp_id("report_id").unwrap();
    let forged_path = f.parent.join("forged-native.json");
    fs::write(&forged_path, forged.canonical()).unwrap();
    let forged_input = f.invoke(&[
        "evidence",
        "native-import",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--report",
        forged_path.to_str().unwrap(),
        "--db",
        db,
    ]);
    assert!(!forged_input.status.success());
    assert!(String::from_utf8_lossy(&forged_input.stderr).contains("Git"));
}

#[test]
fn native_clang_tu_retains_compiler_counts_but_never_manufactures_calls_as_edges() {
    if !available("clang", "--version") {
        return;
    }
    let f = Fixture::new();
    let db_path = f.parent.join("clang-evidence.sqlite");
    let db = db_path.to_str().unwrap();
    let report = parsed(f.invoke(&[
        "analyze",
        "native",
        "--language",
        "c",
        "--source",
        "src/main.c",
        "--compile-db",
        f.db(),
        "--trusted-local",
        "--json",
    ]));
    let path = f.parent.join("native-clang.json");
    fs::write(&path, report.canonical()).unwrap();
    let imported = parsed(f.invoke(&[
        "evidence",
        "native-import",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--db",
        db,
        "--report",
        path.to_str().unwrap(),
    ]));
    let snapshot = field(&imported, "snapshot_id");
    let analyzed = parsed(f.invoke(&["analyze", ".", "--db", db, "--json"]));
    let configured = analyzed
        .get("configured_observations")
        .unwrap()
        .array_ref()
        .unwrap();
    assert!(configured.iter().any(|item|
        field(item, "native_kind") == "NATIVE_CONFIGURED_HEADER" &&
        field(item.get("source").unwrap(), "path") == "src/main.h"
    ), "tracked header should be independently Git-bound");
    assert!(configured.iter().any(|item| {
        if field(item, "native_kind") != "NATIVE_CONFIGURED_TU" {
            return false;
        }
        let payload = item.get("native_payload").unwrap();
        payload.get("symbol_count").unwrap().integer().unwrap() > 0
            && payload.get("call_count").unwrap().integer().unwrap() > 0
    }));
    assert!(
        !analyzed
            .get("relationships")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .any(|item| field(item, "kind") == "CLANG_CALL")
    );
    let replay = parsed(f.invoke(&[
        "evidence",
        "native-store-audit",
        "--repo",
        f.repo.to_str().unwrap(),
        "--rev",
        &f.commit,
        "--snapshot",
        snapshot,
        "--db",
        db,
    ]));
    assert_eq!(
        field(&replay, "status"),
        "REPLAYED_GIT_BOUND_NATIVE_OBSERVATIONS"
    );
    fs::write(
        f.repo.join("src/main.h"),
        "static inline int helper(int n) { return n + 2; }\n",
    ).unwrap();
    ok("git", &["add", "src/main.h"], &f.repo);
    ok("git", &[
        "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "modify tracked Clang header",
    ], &f.repo);
    let new_head = String::from_utf8(ok("git", &["rev-parse", "HEAD"], &f.repo))
        .unwrap().trim().to_owned();
    let impact = parsed(f.invoke(&[
        "impact", ".", "--db", db, "--base", &f.commit,
        "--head", &new_head, "--json",
    ]));
    assert!(impact.get("configured_observations").unwrap().array_ref().unwrap()
        .iter().any(|item|
            field(item, "native_kind") == "NATIVE_CONFIGURED_HEADER" &&
            field(item.get("source").unwrap(), "path") == "src/main.h"
        ), "changed tracked header must surface as configured native evidence");
}
