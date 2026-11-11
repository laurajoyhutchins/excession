//! Native compiler/interpreter dependency observations from a pinned Git checkout.
//! Toolchain execution is trusted-local, not cryptographic worker attestation.
use excession::json::{self, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("launch")
}
fn ok(out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn reject(out: Output, part: &str) {
    assert!(!out.status.success(), "unexpected acceptance");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(part),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|result| result.status.success())
}
fn grammars() -> bool {
    run("python3", &["-c", "import tree_sitter_language_pack"])
        .status
        .success()
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(out: Output) -> Value {
    json::parse(&ok(out)).unwrap()
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).unwrap().str().unwrap()
}
fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).unwrap().array_ref().unwrap()
}
fn paths(v: &Value, key: &str) -> BTreeSet<String> {
    arr(v, key)
        .iter()
        .map(|v| s(v, "path").to_string())
        .collect()
}
fn edges(v: &Value) -> BTreeSet<(String, String)> {
    arr(v, "matching_edges")
        .iter()
        .map(|e| (s(e, "source_path").into(), s(e, "target_path").into()))
        .collect()
}
struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    rev: String,
    syntax: PathBuf,
    resolution: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "excession-inline-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let root = repo.to_str().unwrap();
        ok(run("git", &["-C", root, "init", "-q", "-b", "main"]));
        ok(run(
            "git",
            &["-C", root, "config", "user.email", "test@example.invalid"],
        ));
        ok(run(
            "git",
            &["-C", root, "config", "user.name", "Native tests"],
        ));
        let sources = [
            ("go.mod", "module example.org/inline\n\ngo 1.23\n"),
            (
                "src/go/main.go",
                "package goapp\nimport \"example.org/inline/src/dep\"\nvar A = dep.Answer\n",
            ),
            ("src/dep/dep.go", "package dep\nconst Answer = 42\n"),
            (
                "Cargo.toml",
                "[package]\nname = \"native-inline\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            (
                "src/main.rs",
                "mod math;\nuse crate::math::value;\nfn main() { let _ = value(); let _ = include_str!(\"data.txt\"); }\n",
            ),
            ("src/math.rs", "pub fn value() -> u32 { 42 }\n"),
            ("src/data.txt", "source-bound asset\n"),
            (
                "tsconfig.json",
                "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@/*\":[\"lib/*\"]}}}",
            ),
            (
                "src/ui.ts",
                "import { value } from \"@/math\";\nexport const answer = value;\n",
            ),
            ("lib/math.ts", "export const value = 42;\n"),
            (
                "src/app.js",
                "const {value} = require(\"./helper.js\");\nconsole.log(value);\n",
            ),
            ("src/helper.js", "module.exports = {value: 42};\n"),
        ];
        for (name, content) in sources {
            let target = repo.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
        ok(run("git", &["-C", root, "add", "-A"]));
        ok(run(
            "git",
            &["-C", root, "commit", "-qm", "pinned native fixture"],
        ));
        let rev = String::from_utf8(ok(run("git", &["-C", root, "rev-parse", "HEAD"])))
            .unwrap()
            .trim()
            .to_owned();
        let syntax = base.join("syntax.json");
        let resolution = base.join("resolution.json");
        fs::write(
            &syntax,
            ok(cli(&[
                "syntax", "scan", "--repo", root, "--rev", &rev, "--roots", "src",
            ])),
        )
        .unwrap();
        fs::write(
            &resolution,
            ok(cli(&[
                "syntax",
                "resolve",
                "--repo",
                root,
                "--report",
                syntax.to_str().unwrap(),
            ])),
        )
        .unwrap();
        Self {
            base,
            repo,
            rev,
            syntax,
            resolution,
        }
    }
    fn invoke(&self, entry: &str, language: &str, trust: &str) -> Output {
        let npm = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules/.bin");
        let mut command = Command::new(env!("CARGO_BIN_EXE_excession"));
        command.args([
            "syntax",
            "native-run",
            "--repo",
            self.repo.to_str().unwrap(),
            "--syntax-report",
            self.syntax.to_str().unwrap(),
            "--resolution-report",
            self.resolution.to_str().unwrap(),
            "--entry",
            entry,
            "--language",
            language,
            "--trusted-local",
            trust,
        ]);
        if npm.is_dir() {
            let path = format!(
                "{}:{}",
                npm.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            command.env("PATH", path);
        }
        command.output().unwrap()
    }
    fn validate<'a>(&self, v: &'a Value, language: &str, entry: &str) -> &'a Value {
        assert_eq!(s(v, "schema"), "excession.native-syntax-inline.v1");
        assert_eq!(s(v, "status"), "LOCAL_NATIVE_DISCOVERY_AND_BOUNDARY_CHECK");
        assert_eq!(s(v, "execution"), "TRUSTED_LOCAL_NOT_ATTESTED");
        assert_eq!(s(v.get("stats").unwrap(), "commit"), self.rev);
        let declaration = v.get("declaration").unwrap();
        assert_eq!(s(declaration, "subject"), entry);
        assert_eq!(s(declaration, "language"), language);
        assert_eq!(s(declaration, "coverage"), "DECLARED_INPUTS_ONLY");
        let boundary = v.get("boundary").unwrap();
        assert_eq!(s(boundary, "source_commit"), self.rev);
        assert_eq!(s(boundary, "declaration_id"), declaration.sha256());
        boundary
    }
}
#[test]
fn rust_native_resolver_is_replaced_by_rust_analyzer_scip() {
    if !grammars() { return; }
    let f = Fixture::new();
    reject(f.invoke("src/main.rs", "rust", "true"), "rust-analyzer SCIP");
}
#[test]
fn go_compiler_dependency_graph_is_checked_against_source() {
    if !grammars() || !available("go") {
        return;
    }
    let f = Fixture::new();
    let observed = data(f.invoke("src/go/main.go", "go", "true"));
    let boundary = f.validate(&observed, "go", "src/go/main.go");
    let found = paths(boundary, "pinned_inputs");
    assert!(
        ["go.mod", "src/go/main.go", "src/dep/dep.go"]
            .iter()
            .all(|p| found.contains(*p))
    );
    assert!(edges(boundary).contains(&("src/go/main.go".into(), "src/dep/dep.go".into())));
}
#[test]
fn typescript_alias_is_native_only_not_silently_promoted() {
    let npm = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules/.bin/tsc");
    if !grammars() || !available("node") || !npm.exists() {
        return;
    }
    let f = Fixture::new();
    let observed = data(f.invoke("src/ui.ts", "typescript", "true"));
    let boundary = f.validate(&observed, "typescript", "src/ui.ts");
    assert!(paths(boundary, "pinned_inputs").contains("lib/math.ts"));
    assert!(arr(boundary, "native_only_sources").contains(&Value::string("lib/math.ts")));
    assert!(arr(boundary, "unscanned_sources").contains(&Value::string("lib/math.ts")));
}
#[test]
fn javascript_require_and_explicit_trust_gate() {
    let npm = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules/.bin/tsc");
    if !grammars() || !available("node") || !npm.exists() {
        return;
    }
    let f = Fixture::new();
    let observed = data(f.invoke("src/app.js", "javascript", "true"));
    let boundary = f.validate(&observed, "javascript", "src/app.js");
    assert!(paths(boundary, "pinned_inputs").contains("src/helper.js"));
    reject(
        f.invoke("src/app.js", "javascript", "false"),
        "--trusted-local true",
    );
}
#[test]
fn forged_resolution_revision_is_refused_before_native_invocation() {
    if !grammars() {
        return;
    }
    let f = Fixture::new();
    let mut resolution = json::parse(&fs::read(&f.resolution).unwrap()).unwrap();
    resolution
        .entries_mut()
        .unwrap()
        .insert("source_commit".into(), Value::string("f".repeat(40)));
    resolution.entries_mut().unwrap().remove("resolution_id");
    let digest = resolution.sha256();
    resolution
        .entries_mut()
        .unwrap()
        .insert("resolution_id".into(), Value::string(digest));
    fs::write(&f.resolution, resolution.canonical()).unwrap();
    reject(
        f.invoke("src/main.rs", "rust", "true"),
        "mismatched syntax/resolution identities",
    );
}
#[test]
fn dirty_checkout_is_refused_before_compiler_execution() {
    if !grammars() {
        return;
    }
    let f = Fixture::new();
    fs::write(f.repo.join("extra.rs"), "fn extra() {}\n").unwrap();
    reject(f.invoke("src/main.rs", "rust", "true"), "dirty checkout");
}
