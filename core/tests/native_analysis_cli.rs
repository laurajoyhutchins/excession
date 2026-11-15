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
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.parent); }
}
fn run(program: &str, args: &[&str], dir: &Path) -> Output {
    Command::new(program).args(args).current_dir(dir).output().unwrap()
}
fn ok(program: &str, args: &[&str], dir: &Path) -> Vec<u8> {
    let output = run(program, args, dir);
    assert!(output.status.success(), "{program}: {}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}
fn available(program: &str, arg: &str) -> bool {
    Command::new(program).arg(arg).output().is_ok_and(|v| v.status.success())
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
            ("src/main.c", "#include \"main.h\"\nint entry(int a) { return helper(a); }\n"),
            ("src/main.h", "static inline int helper(int n) { return n + 1; }\n"),
            ("src/worker.cpp", "namespace app { int twice(int n) { return n+n; } }\n"),
        ] {
            let file = repo.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, source).unwrap();
        }
        ok("git", &["add", "."], &repo);
        ok("git", &[
            "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-qm", "source",
        ], &repo);
        let commit = String::from_utf8(ok("git", &["rev-parse", "HEAD"], &repo))
            .unwrap().trim().to_string();
        let clang_db = parent.join("compile_commands.json");
        let cwd = repo.display().to_string();
        let db = Value::array([
            excession::json::obj(&[
                ("directory", Value::string(&cwd)),
                ("file", Value::string("src/main.c")),
                ("arguments", Value::array([
                    Value::string("clang"), Value::string("-c"),
                    Value::string("src/main.c"),
                ])),
            ]),
            excession::json::obj(&[
                ("directory", Value::string(&cwd)),
                ("file", Value::string("src/worker.cpp")),
                ("arguments", Value::array([
                    Value::string("clang++"), Value::string("-std=c++17"),
                    Value::string("-c"), Value::string("src/worker.cpp"),
                ])),
            ]),
        ]);
        fs::write(&clang_db, db.canonical()).unwrap();
        Self { parent, repo, clang_db, commit }
    }
    fn invoke(&self, args: &[&str]) -> Output {
        run(env!("CARGO_BIN_EXE_excession"), args, &self.repo)
    }
    fn db(&self) -> &str { self.clang_db.to_str().unwrap() }
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).unwrap().str().unwrap()
}
fn parsed(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    json::parse(&output.stdout).unwrap()
}
#[test]
fn native_go_inputs_have_git_blobs_and_no_fabricated_call_graph() {
    if !available("go", "version") || !available("python3", "--version") { return; }
    let f = Fixture::new();
    let response = parsed(f.invoke(&[
        "analyze", "native", "--language", "go", "--source", "cmd/server/main.go",
        "--trusted-local", "--json",
    ]));
    assert_eq!(field(&response, "schema"), "excession.native-analysis.v1");
    assert_eq!(field(&response, "scope"), "COMPILER_SELECTED_INPUTS_ONLY");
    assert_eq!(field(&response, "source_commit"), f.commit);
    assert!(response.content_id_matches("report_id").unwrap());
    assert_eq!(field(response.get("source").unwrap(), "path"), "cmd/server/main.go");
    let inputs = response.get("inputs").unwrap().array_ref().unwrap();
    let paths = inputs.iter().map(|e|
        field(e.get("source").unwrap(), "path")
    ).collect::<Vec<_>>();
    for expected in ["cmd/server/main.go", "dep/dep.go", "go.mod"] {
        assert!(paths.contains(&expected), "{expected} not found in {paths:?}");
    }
    for record in inputs {
        let item = record.get("source").unwrap();
        let path = field(item, "path");
        let expected = String::from_utf8(ok("git", &["rev-parse", &format!("HEAD:{path}")], &f.repo)).unwrap();
        assert_eq!(field(item, "blob"), expected.trim());
        assert_eq!(field(item, "source_tree"), field(&response, "source_tree"));
    }
    assert_eq!(field(&response, "execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
    assert!(response.get("observation").unwrap().get("imports").is_err());
    let human = f.invoke(&[
        "analyze", "native", "--language", "go",
        "--source", "cmd/server/main.go", "--trusted-local",
    ]);
    assert!(human.status.success(), "{}", String::from_utf8_lossy(&human.stderr));
    assert!(String::from_utf8_lossy(&human.stdout).contains("COMPILER_SELECTED_INPUTS_ONLY"));
}

#[test]
fn native_c_and_cpp_preserve_configured_ast_and_unverified_header_scope() {
    if !available("clang", "--version") || !available("clang++", "--version") { return; }
    let f = Fixture::new();
    for (language, source, expected) in [
        ("c", "src/main.c", "entry"),
        ("cpp", "src/worker.cpp", "twice"),
    ] {
        let response = parsed(f.invoke(&[
            "analyze", "native", "--language", language, "--source", source,
            "--compile-db", f.db(), "--trusted-local", "--json",
        ]));
        assert!(response.content_id_matches("report_id").unwrap());
        assert_eq!(field(&response, "scope"), "CONFIGURED_TU_NOT_BUILD_VERIFIED");
        assert_eq!(field(&response, "source_commit"), f.commit);
        let native = response.get("observation").unwrap().get("observation").unwrap();
        let symbols = native.get("symbols").unwrap().array_ref().unwrap();
        assert!(symbols.iter().any(|s| field(s, "name") == expected), "{source}: {symbols:?}");
        assert_eq!(field(&response, "execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
        assert!(response.get("limitations").unwrap().array_ref().unwrap().iter().any(|l|
            l.str().unwrap().contains("not fully Git-authenticated")
        ));
    }
}

#[test]
fn native_refuses_unsupported_languages_dirty_worktrees_and_wrong_revision() {
    let f = Fixture::new();
    for extra in [
        vec!["--language", "java", "--source", "src/Main.java", "--trusted-local"],
        vec!["--language", "cpp", "--source", "src/worker.cpp", "--trusted-local"],
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
        "analyze", "native", "--language", "c", "--source", "src/main.c",
        "--compile-db", f.db(), "--trusted-local",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("dirty checkout"));
}
