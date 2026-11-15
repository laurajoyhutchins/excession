//! Cross-language CLI contract: one Git-pinned parser/coverage surface with
//! no fabricated semantic edges for languages without a resolver.
use excession::{doctor, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn command(name: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(name).args(args).current_dir(cwd).output().unwrap()
}
fn checked(name: &str, args: &[&str], cwd: &Path) -> Vec<u8> {
    let out = command(name, args, cwd);
    assert!(out.status.success(), "{}: {}", name, String::from_utf8_lossy(&out.stderr));
    out.stdout
}
struct Fixture { root: PathBuf, repo: PathBuf }
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("excession-lang-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let repo = root.join("source");
        fs::create_dir_all(&repo).unwrap();
        checked("git", &["init", "-q"], &repo);
        for (name, contents) in [
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "import pkg.b\n"),
            ("pkg/b.py", "ANSWER = 42\n"),
            ("web/view.tsx", "import { label } from './label'; export function View() { return <div>{label}</div>; }\n"),
            ("web/label.ts", "export const label: string = 'ok';\n"),
            ("src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            ("cmd/main.go", "package main\nimport \"fmt\"\nfunc main() { fmt.Println(42) }\n"),
            ("native/main.c", "int answer(void) { return 42; }\n"),
            ("go.mod", "module example.org/local\n\ngo 1.23\n"),
        ] {
            let path = repo.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        checked("git", &["add", "."], &repo);
        checked("git", &[
            "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-qm", "mixed sources",
        ], &repo);
        Self { root, repo }
    }
    fn path(&self) -> &str { self.repo.to_str().unwrap() }
}
#[test]
fn cross_language_command_preserves_scope_and_unsupported_coverage() {
    let f = Fixture::new();
    let exe = env!("CARGO_BIN_EXE_excession");
    let output = checked(exe, &["analyze", "languages", "--repo", f.path(), "--json"], &f.repo);
    let report = json::parse(&output).unwrap();
    assert_eq!(report.get("schema").unwrap().str().unwrap(), "excession.language-analysis.v1");
    assert_eq!(report.get("scanned_sources").unwrap().integer().unwrap(), 8);
    assert_eq!(report.get("status").unwrap().str().unwrap(), "PARTIAL");
    let langs = report.get("languages").unwrap().array_ref().unwrap();
    let rust = langs.iter().find(|l| l.get("language").unwrap().str().unwrap() == "rust").unwrap();
    assert_eq!(rust.get("resolution_scope").unwrap().str().unwrap(), "SYNTAX_ONLY_NO_DEPENDENCY_RESOLVER");
    assert_eq!(rust.get("candidate_edges").unwrap().integer().unwrap(), 0);
    for candidate in report.get("candidate_edges").unwrap().array_ref().unwrap() {
        assert_ne!(candidate.get("language").unwrap().str().unwrap(), "rust");
    }
    let selected = checked(exe, &["analyze", "languages", "--repo", f.path(), "--roots", "web", "--json"], &f.repo);
    let selected = json::parse(&selected).unwrap();
    assert_eq!(selected.get("scanned_sources").unwrap().integer().unwrap(), 2);
    assert!(selected.get("languages").unwrap().array_ref().unwrap().iter().all(|l| {
        matches!(l.get("language").unwrap().str().unwrap(), "typescript" | "tsx")
    }));
    let human = String::from_utf8(checked(exe, &["analyze", "languages", "--repo", f.path()], &f.repo)).unwrap();
    assert!(human.contains("Coverage: PARTIAL"));
    assert!(human.contains("rust:"));
}

#[test]
fn ordinary_static_index_keeps_python_and_jsx_tsx_independent() {
    let capabilities = doctor::report();
    if !capabilities.get("capabilities").unwrap().get("javascript_index").unwrap().bool().unwrap()
        || !capabilities.get("capabilities").unwrap().get("python_index").unwrap().bool().unwrap()
    { return; }
    let f = Fixture::new();
    fs::write(f.repo.join("pkg/a.js"), "import { ANSWER } from './b.js';\n").unwrap();
    fs::write(f.repo.join("pkg/b.js"), "export const ANSWER = 42;\n").unwrap();
    fs::write(f.repo.join("pkg/a.jsx"), "export const A = <span/>;\n").unwrap();
    checked("git", &["add", "-A"], &f.repo);
    checked("git", &[
        "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-qm", "JS source collides with Python basename",
    ], &f.repo);

    let db = f.root.join("index.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["analyze", ".", "--db"])
        .arg(&db)
        .arg("--json")
        .current_dir(&f.repo)
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report = json::parse(&output.stdout).unwrap();
    let edges = report.get("relationships").unwrap().array_ref().unwrap();
    let has_edge = |from: &str, to: &str| edges.iter().any(|e| {
        e.get("source").unwrap().get("path").unwrap().str().unwrap() == from &&
        e.get("target").unwrap().get("path").unwrap().str().unwrap() == to
    });
    assert!(has_edge("pkg/a.py", "pkg/b.py"), "Python import missing");
    assert!(has_edge("pkg/a.js", "pkg/b.js"), "JavaScript import missing");
    assert!(has_edge("web/view.tsx", "web/label.ts"), "TSX import missing");
}
