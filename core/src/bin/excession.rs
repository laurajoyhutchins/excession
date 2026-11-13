//! Excession: the only installable binary, backed by one Rust Git/SQLite authority.
//! All commands use one clap grammar; the same parsed options reach the native handlers.

use excession::agent;
use excession::analyze;
use excession::change_inventory;
use excession::discovery;
use excession::doctor;
use excession::evidence_query;
use excession::explain;
use excession::impact_query;
use excession::native_test_selectors;
use excession::native_test_workflow;
use excession::working_tree_tests;
use excession::observed_relations::{self, Plan, Selection};
use excession::semantic_contract;
use excession::sqlite::{Db, Param};
use excession::{
    Result, benchmark, compiler_clang, git, graph_assembly, graph_baseline, index, kbuild_graph,
    kernel, kernel_coverage, native_boundary, preflight, projection, refactor, refactor_codegen,
    refactor_rust_analyzer, refactor_rust_repair, refuse, scip_evidence, scip_refactor,
    security_ingest, syntax, syntax_resolution, views,
};
use std::env;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

#[path = "excession/cli.rs"]
mod cli;
#[path = "excession/graph.rs"]
mod graph_cli;
#[path = "excession/scan.rs"]
mod scan;
use cli::{Options, ParseOutcome};

enum Response {
    Json(excession::json::Value),
    Test(excession::json::Value, bool),
    CanonicalJson(String),
    Message(String),
    Bytes(Vec<u8>),
}
impl Response {
    fn emit(self) -> Result<()> {
        let mut out = io::stdout().lock();
        let write_error = |error: io::Error| format!("stdout write failed: {error}");
        match self {
            Self::Json(j) => writeln!(out, "{}", j.canonical()).map_err(write_error),
            Self::Test(report, as_json) => {
                let phase = report.get("status")?.str()?.to_owned();
                let message = if as_json {
                    report.canonical()
                } else {
                    test_summary(&report)?
                };
                writeln!(out, "{message}").map_err(write_error)?;
                out.flush().map_err(write_error)?;
                if phase == "FAIL" {
                    std::process::exit(1);
                }
                if phase == "INCONCLUSIVE" {
                    std::process::exit(3);
                }
                Ok(())
            }
            Self::CanonicalJson(s) | Self::Message(s) => writeln!(out, "{s}").map_err(write_error),
            Self::Bytes(bytes) => out.write_all(&bytes).map_err(write_error),
        }
    }
}
fn test_summary(report: &excession::json::Value) -> Result<String> {
    use excession::json::Value;
    let provider = match report.get("provider")? {
        Value::Null => "not resolved",
        value => value.str()?,
    };
    let phase = report.get("status")?.str()?;
    let mode = report.get("mode")?.str()?;
    let base = report.get("base_commit")?.str()?;
    let head = report.get("candidate_commit")?.str()?;
    let short = |oid: &str| oid.chars().take(12).collect::<String>();
    let changed = report.get("changed_sources")?.array_ref()?.len();
    let worktree = report.entries()?.get("working_tree");
    let repo_path = report.get("repo")?.str()?;
    let mut lines = vec![
        format!("Tests: {phase} ({mode}, {provider})"),
        format!("Repository: {repo_path}"),
        format!("Source: {}", if worktree.is_some() { "ephemeral uncommitted snapshot" } else { "committed Git revision" }),
        format!(
            "Revision: {}..{} | {changed} changed file(s)",
            short(base),
            short(head)
        ),
    ];
    if let Some(snapshot) = worktree {
        lines.push(format!(
            "Snapshot: {} | original checkout stays untouched",
            snapshot.get("snapshot_sha256")?.str()?.chars().take(12).collect::<String>()
        ));
    }
    for reason in report.get("blockers")?.array_ref()? {
        lines.push(format!("  Blocked: {}", reason.str()?));
    }
    if mode == "PLAN" {
        if phase == "BLOCKED" {
            lines.push(format!("Fix: {}", report.get("guidance")?.str()?));
        }
        lines.push(format!("Next: {}", report.get("next_command")?.str()?));
        if phase == "READY" {
            lines.push("This is a read-only plan. --run executes native project tools.".to_owned());
        }
    } else {
        let selected = report.get("selected_test_files")?.array_ref()?;
        lines.push(format!(
            "Selected: {} test file(s) | Executed: {} test(s)",
            selected.len(),
            report.get("executed_test_count")?.integer()?
        ));
        for path in selected.iter().take(8) {
            lines.push(format!("  {}", path.str()?));
        }
        if selected.len() > 8 {
            lines.push(format!("  ... and {} more", selected.len() - 8));
        }
        if report.get("verification")? != &Value::Null {
            lines.push(format!(
                "Excession verification: {}",
                report.get("verification")?.get("status")?.str()?
            ));
        }
    }
    lines.push("Coverage remains partial; unlisted tests are not proven safe to skip.".to_owned());
    Ok(lines.join("\n"))
}

fn json(v: excession::json::Value) -> Response {
    Response::Json(v)
}

/// REA is artifact-first. Build provenance is a separately verified link, not
/// a fabricated source identity for binaries with unavailable source.
fn rea_build_link<'a>(opt: &'a Options) -> Result<Option<excession::evidence::rea::BuildLink<'a>>> {
    match (
        opt.0.get("repo"),
        opt.0.get("rev"),
        opt.0.get("build-snapshot"),
    ) {
        (None, None, None) => Ok(None),
        (Some(repo), Some(revision), Some(snapshot)) => {
            Ok(Some(excession::evidence::rea::BuildLink {
                repo: Path::new(repo),
                revision,
                snapshot,
            }))
        }
        _ => refuse("REA build provenance needs --repo, --rev and --build-snapshot together"),
    }
}

fn view_cmd(verb: &str, opt: &Options, replay: bool) -> Result<Response> {
    if (verb, replay) == ("deps", false) {
        let db = Db::open(opt.path("db"))?;
        return Ok(Response::CanonicalJson(projection::dependencies_canonical(
            &db,
            opt.get("name"),
            opt.get("rev"),
            opt.get("seed"),
        )?));
    }
    let v = match (verb, replay) {
        ("deps", true) => {
            let db = Db::open(opt.path("db"))?;
            let plan =
                projection::dependencies(&db, opt.get("name"), opt.get("rev"), opt.get("seed"))?;
            projection::replay(&plan, opt.path("manifest"))?
        }
        ("profile-deps", false) => {
            let db = Db::open(opt.path("db"))?;
            projection::profile_dependencies(&db, opt.get("name"), opt.get("rev"), opt.get("seed"))?
        }
        ("graph", false) => {
            let db = Db::open(opt.path("db"))?;
            graph_baseline::paths(
                &db,
                opt.get("name"),
                opt.get("rev"),
                opt.get("seed"),
                opt.get("kind"),
            )?
        }
        ("paths", false) => {
            let db = Db::open(opt.path("db"))?;
            projection::dependency_paths(
                &db,
                opt.get("name"),
                opt.get("rev"),
                opt.get("seed"),
                opt.get("role"),
            )?
        }
        ("security", false) | ("security", true) => {
            let db = Db::open(opt.path("db"))?;
            let plan = projection::security(&db, opt.path("repo"), opt.get("run"))?;
            if replay {
                projection::replay(&plan, opt.path("manifest"))?
            } else {
                plan
            }
        }
        ("compose", false) | ("compose", true) => {
            if replay {
                projection::compose_replay(
                    opt.path("repo"),
                    opt.get("op"),
                    opt.path("left"),
                    opt.path("right"),
                    opt.path("output"),
                )?
            } else {
                projection::compose(
                    opt.path("repo"),
                    opt.get("op"),
                    opt.path("left"),
                    opt.path("right"),
                )?
            }
        }
        ("select", false) | ("select", true) => {
            if replay {
                projection::select_replay(
                    opt.path("repo"),
                    opt.path("manifest"),
                    opt.get("axis"),
                    opt.get("value"),
                    opt.path("output"),
                )?
            } else {
                projection::select(
                    opt.path("repo"),
                    opt.path("manifest"),
                    opt.get("axis"),
                    opt.get("value"),
                )?
            }
        }
        ("mount", false) | ("mount", true) => {
            if replay {
                projection::mount_replay(
                    opt.path("repo"),
                    opt.path("manifest"),
                    opt.get("namespace"),
                    opt.path("output"),
                )?
            } else {
                projection::mount(opt.path("repo"), opt.path("manifest"), opt.get("namespace"))?
            }
        }
        ("ls", false) => projection::list(
            opt.path("repo"),
            opt.path("manifest"),
            opt.get_or("path", ""),
        )?,
        ("stat", false) => {
            projection::describe(opt.path("repo"), opt.path("manifest"), opt.get("path"))?
        }
        ("cat", false) => {
            return Ok(Response::Bytes(projection::open(
                opt.path("repo"),
                opt.path("manifest"),
                opt.get("path"),
            )?));
        }
        ("materialize", false) | ("audit", false) => {
            if verb == "materialize" {
                projection::materialize(opt.path("repo"), opt.path("manifest"), opt.path("output"))?
            } else {
                projection::audit(opt.path("repo"), opt.path("manifest"), opt.path("output"))?
            }
        }
        _ => {
            return refuse(format!(
                "unknown view command {verb:?}; see excession --help"
            ));
        }
    };
    Ok(json(v))
}

fn inspect_cmd(verb: &str, opt: &Options) -> Result<Response> {
    let db = Db::open(opt.path("db"))?;
    let resolve = |spec: &str| -> Result<String> {
        let root = db.scalar(
            "SELECT git_dir FROM repository WHERE name=?",
            &[Param::Text(opt.get("name"))],
        )?;
        git::rev(Path::new(&root), spec)
    };
    Ok(json(match verb {
        "dependencies" => views::dependencies(&db, opt.get("name"), &resolve(opt.get("rev"))?)?,
        "manifest" => views::manifest(&db, opt.get("name"), &resolve(opt.get("rev"))?)?,
        "impact" => views::impact(
            &db,
            opt.get("name"),
            &resolve(opt.get("rev"))?,
            opt.get("seed"),
        )?,
        "review" => views::review(
            &db,
            opt.get("name"),
            &resolve(opt.get("old"))?,
            &resolve(opt.get("new"))?,
        )?,
        _ => unreachable!(),
    }))
}

fn parse_browser_path(current: &str, entered: &str) -> Result<String> {
    let mut segments = if entered.starts_with('/') {
        Vec::new()
    } else {
        current
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    for part in entered.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                if segments.pop().is_none() {
                    return refuse("cannot browse above view root");
                }
            }
            item => segments.push(item.to_owned()),
        }
    }
    Ok(segments.join("/"))
}

fn safe_display(bytes: &[u8], max: usize) -> String {
    let mut result = String::new();
    for ch in String::from_utf8_lossy(&bytes[..bytes.len().min(max)]).chars() {
        if matches!(ch, '\n' | '\t') || !ch.is_control() {
            result.push(ch);
        } else {
            result.push_str(&format!("\\u{{{:04x}}}", ch as u32));
        }
    }
    if bytes.len() > max {
        result.push_str(&format!("\n[display truncated at {max} bytes]"));
    }
    result
}

fn browse(repo: &Path, manifest: &Path, max: usize) -> Result<()> {
    projection::list(repo, manifest, "")?; // authenticate before opening REPL
    let interactive = io::stdin().is_terminal();
    let stdin = io::stdin();
    let mut cwd = String::new();
    if interactive {
        println!("Excession read-only Git browser. Commands: ls cd pwd stat show help quit");
    }
    let mut input = stdin.lock();
    loop {
        if interactive {
            print!("excession:/{cwd}> ");
            io::stdout().flush().map_err(|e| e.to_string())?;
        }
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let command = line.trim();
        if command.is_empty() {
            continue;
        }
        let (verb, target) = command.split_once(' ').unwrap_or((command, ""));
        let target = target.trim();
        if matches!(verb, "quit" | "exit") {
            break;
        }
        let response = match verb {
            "help" if target.is_empty() => {
                "Commands: ls [PATH], cd PATH, pwd, stat FILE, show FILE, quit".to_owned()
            }
            "pwd" if target.is_empty() => format!("/{cwd}"),
            "ls" => {
                let path = parse_browser_path(&cwd, if target.is_empty() { "." } else { target })?;
                let listing = projection::list(repo, manifest, &path)?;
                listing
                    .get("children")?
                    .array_ref()?
                    .iter()
                    .map(|v| {
                        Ok(format!(
                            "{}{}",
                            safe_display(v.get("name")?.str()?.as_bytes(), 256),
                            if v.get("type")?.str()? == "directory" {
                                "/"
                            } else {
                                ""
                            }
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("\n")
            }
            "cd" if !target.is_empty() => {
                let path = parse_browser_path(&cwd, target)?;
                projection::list(repo, manifest, &path)?;
                cwd = path;
                format!("/{cwd}")
            }
            "stat" if !target.is_empty() => {
                let path = parse_browser_path(&cwd, target)?;
                projection::describe(repo, manifest, &path)?.canonical()
            }
            "show" if !target.is_empty() => {
                let path = parse_browser_path(&cwd, target)?;
                safe_display(&projection::open(repo, manifest, &path)?, max)
            }
            _ => return refuse(format!("invalid browser command {verb:?}; see help")),
        };
        println!("{response}");
    }
    Ok(())
}

fn clang_command(opt: &Options) -> Result<Response> {
    let default_compiler = if opt.get("source").ends_with(".c") {
        "clang"
    } else {
        "clang++"
    };
    Ok(json(compiler_clang::scan(
        opt.path("repo"),
        opt.get_or("rev", "HEAD"),
        opt.get("source"),
        opt.path("compile-db"),
        opt.get_or("compiler", default_compiler),
        compiler_clang::ClangOptions {
            config: opt.0.get("config").map(|s| Path::new(s.as_str())),
            object_prefix: opt.0.get("object-prefix").map(String::as_str),
        },
        opt.get("trusted-local"),
    )?))
}

fn execute(args: &[String]) -> Result<Response> {
    let parsed = match cli::parse(args)? {
        ParseOutcome::Help(message) => return Ok(Response::Message(message)),
        ParseOutcome::Command(parsed) => parsed,
    };
    let opt = &parsed.options;
    match parsed.path.as_slice() {
        [top, sub] if top == "semantic" => match sub.as_str() {
            "scan" => {
                let roots = opt.get("roots").split(',').collect::<Vec<_>>();
                Ok(json(semantic_contract::scan(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    &roots,
                )?))
            }
            "audit" => {
                let roots = opt.get("roots").split(',').collect::<Vec<_>>();
                Ok(json(semantic_contract::audit_scan(
                    opt.path("repo"),
                    &roots,
                    opt.path("report"),
                )?))
            }
            "resolve" => Ok(json(semantic_contract::resolve(
                opt.path("repo"),
                opt.path("syntax-report"),
            )?)),
            "export" => Ok(json(semantic_contract::export(
                opt.path("repo"),
                opt.path("syntax-report"),
                opt.path("resolution-report"),
            )?)),
            _ => refuse(format!("unknown semantic command {sub}")),
        },
        [top, sub] if top == "syntax" => match sub.as_str() {
            "scan" => {
                let roots = opt.get("roots").split(',').collect::<Vec<_>>();
                Ok(json(syntax::scan(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    &roots,
                )?))
            }
            "audit" => Ok(json(syntax::audit(opt.path("repo"), opt.path("report"))?)),
            "preflight" => Ok(json(preflight::syntax_file(opt.path("report"))?)),
            "resolve-preflight" => Ok(json(preflight::resolution_file(opt.path("report"))?)),
            "native-run" => {
                let trusted = match opt.get("trusted-local") {
                    "true" => true,
                    "false" => false,
                    _ => return refuse("--trusted-local requires true or false"),
                };
                Ok(json(native_boundary::discover_and_compare(
                    opt.path("repo"),
                    opt.path("syntax-report"),
                    opt.path("resolution-report"),
                    opt.get("entry"),
                    opt.get("language"),
                    opt.0.get("import-roots").map(String::as_str),
                    trusted,
                )?))
            }
            "native-preflight" => Ok(json(native_boundary::preflight_file(
                opt.path("declaration"),
            )?)),
            "native-compare" => Ok(json(native_boundary::compare(
                opt.path("repo"),
                opt.path("syntax-report"),
                opt.path("resolution-report"),
                opt.path("declaration"),
            )?)),
            "native-audit" => Ok(json(native_boundary::audit(
                opt.path("repo"),
                opt.path("syntax-report"),
                opt.path("resolution-report"),
                opt.path("declaration"),
                opt.path("report"),
            )?)),
            "resolve" => Ok(json(syntax_resolution::candidates(
                opt.path("repo"),
                opt.path("report"),
            )?)),
            "resolve-audit" => Ok(json(syntax_resolution::audit(
                opt.path("repo"),
                opt.path("syntax-report"),
                opt.path("report"),
            )?)),
            _ => refuse(format!("unknown syntax command {sub}")),
        },
        [top, sub] if top == "graph" => match sub.as_str() {
            "import" | "ls" | "show" | "deps" | "path" => graph_cli::run(sub, opt),
            "kbuild" => {
                let db = Db::open(opt.path("db"))?;
                Ok(json(kbuild_graph::assemble(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    opt.path("build"),
                    opt.get("archive"),
                    opt.get_or("ar", "ar"),
                    opt.get("trusted-local"),
                    &db,
                )?))
            }
            "assemble" => {
                let db = Db::open(opt.path("db"))?;
                Ok(json(graph_assembly::assemble(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    opt.path("build"),
                    opt.get("target"),
                    Some(opt.path("observations")),
                    &db,
                )?))
            }
            _ => refuse(format!("unknown graph command: {sub}")),
        },
        [top] if top == "tests" || top == "test" => {
            let repo = Path::new(opt.get_or("repo", "."));
            let explicit_worktree = opt.get_or("working-tree", "false") == "true";
            let sandbox_image = opt.0.get("sandbox-image").map(String::as_str);
            let expected_snapshot = opt.0.get("expected-snapshot").map(String::as_str);
            let timeout_seconds = opt.get_or("timeout", "120").parse::<u64>()
                .map_err(|_| "--timeout must be an integer from 1 to 600")?;
            let dirty_source = working_tree_tests::has_source_changes(repo)?;
            if explicit_worktree || dirty_source {
                let report = working_tree_tests::execute(working_tree_tests::Request {
                    repo,
                    base: opt.0.get("base").map(String::as_str),
                    candidate: opt.0.get("candidate").map(String::as_str),
                    provider: opt.0.get("provider").map(String::as_str),
                    tool: opt.0.get("tool").map(String::as_str),
                    focus: opt.0.get("focus").map(String::as_str),
                    run: opt.get_or("run", "false") == "true",
                    verify: opt.get_or("verify", "false") == "true",
                    sandbox_image,
                    expected_snapshot,
                    timeout_seconds,
                })?;
                return Ok(Response::Test(report, opt.get_or("json", "false") == "true"));
            }
            if sandbox_image.is_some() || expected_snapshot.is_some() {
                return refuse("sandbox flags require uncommitted source changes or --working-tree");
            }
            let report = native_test_workflow::execute(native_test_workflow::Request {
                repo: Path::new(opt.get_or("repo", ".")),
                base: opt.0.get("base").map(String::as_str),
                candidate: opt.0.get("candidate").map(String::as_str),
                provider: opt.0.get("provider").map(String::as_str),
                tool: opt.0.get("tool").map(String::as_str),
                focus: opt.0.get("focus").map(String::as_str),
                run: opt.get_or("run", "false") == "true",
                verify: opt.get_or("verify", "false") == "true",
            })?;
            Ok(Response::Test(
                report,
                opt.get_or("json", "false") == "true",
            ))
        }
        [top, sub] if top == "agent" => {
            let limit = opt
                .get_or("limit", "32")
                .parse::<usize>()
                .map_err(|_| "--limit must be an integer from 1 to 64")?;
            let roots = opt.0.get("roots").map(String::as_str);
            let db = opt.0.get("db").map(String::as_str);
            match sub.as_str() {
                "context" => Ok(json(agent::context(
                    opt.path("repo"),
                    opt.get("rev"),
                    opt.get("focus"),
                    opt.0.get("task").map(String::as_str),
                    limit,
                    roots,
                    db,
                )?)),
                "verify" => Ok(json(agent::verify(
                    opt.path("repo"),
                    opt.get("base"),
                    opt.get("candidate"),
                    limit,
                    roots,
                    db,
                    opt.0.get("receipts").map(Path::new),
                )?)),
                "select-tests" => Ok(json(native_test_selectors::run(
                    opt.path("repo"),
                    opt.get("base"),
                    opt.get("candidate"),
                    opt.get("provider"),
                    opt.0.get("tool").map(String::as_str),
                    opt.0.get("focus").map(String::as_str),
                    opt.get("trusted-local"),
                )?)),
                _ => refuse(format!("unknown agent command: {sub}")),
            }
        }
        [top, sub] if top == "analyze" && sub == "clang" => clang_command(opt),
        [top] if top == "analyze" => {
            let repo = Path::new(opt.get_or("path", "."));
            let rev = opt.get_or("rev", "HEAD");
            let roots = opt.0.get("roots").map(String::as_str);
            let db = opt.0.get("db").map(String::as_str);
            let explicit = ["scip-snapshot", "graph-id", "source-prefix"]
                .iter()
                .any(|key| opt.0.contains_key(*key));
            let (report, db_path) = if explicit {
                analyze::run_with_evidence(
                    repo,
                    rev,
                    roots,
                    db,
                    Selection {
                        scip_snapshot: opt.0.get("scip-snapshot").map(String::as_str),
                        graph_id: opt.0.get("graph-id").map(String::as_str),
                        source_prefix: opt.0.get("source-prefix").map(String::as_str),
                    },
                )?
            } else {
                analyze::run_default(repo, rev, roots, db)?
            };
            if opt.get_or("json", "false") == "true" {
                Ok(json(report))
            } else {
                Ok(Response::Message(analyze::human(&report, &db_path)?))
            }
        }
        [top, sub] if top == "kernel" => match sub.as_str() {
            "scan" => {
                let roots = opt.get("roots").split(',').collect::<Vec<_>>();
                Ok(json(kernel::scan(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    &roots,
                )?))
            }
            "inventory" | "partition" | "audit" => {
                let size = opt
                    .get_or("size", "128")
                    .parse::<usize>()
                    .map_err(|_| "invalid kernel partition size")?;
                let repo = opt.path("repo");
                let rev = opt.get_or("rev", "HEAD");
                match sub.as_str() {
                    "inventory" => Ok(json(kernel::inventory(repo, rev, size)?)),
                    "partition" => {
                        let index = opt
                            .get("index")
                            .parse::<usize>()
                            .map_err(|_| "invalid kernel partition index")?;
                        Ok(json(kernel::scan_partition(repo, rev, index, size)?))
                    }
                    _ => Ok(json(kernel_coverage::audit(
                        repo,
                        rev,
                        opt.path("dir"),
                        size,
                    )?)),
                }
            }
            "clang" => clang_command(opt),
            _ => refuse(format!("unknown kernel command: {sub}")),
        },
        [top] if top == "discover" => {
            let found = discovery::inspect(
                Path::new(opt.get_or("path", ".")),
                opt.get_or("rev", "HEAD"),
            )?;
            Ok(json(found.report()?))
        }
        [top] if top == "explain" => {
            let repo = Path::new(opt.get_or("path", "."));
            let report = Path::new(opt.get("report"));
            let claim = opt.get("id");
            let roots = opt.0.get("roots").map(String::as_str);
            let db = opt.0.get("db").map(String::as_str);
            if opt.get_or("audit", "false") == "true" {
                Ok(json(explain::audit(repo, report, claim, roots, db)?))
            } else {
                let result = explain::explain(repo, report, claim, roots, db)?;
                if opt.get_or("json", "false") == "true" {
                    Ok(json(result))
                } else {
                    Ok(Response::Message(explain::human(&result)?))
                }
            }
        }
        [top] if top == "impact" => {
            let repo = Path::new(opt.get_or("path", "."));
            let base = opt.get("base");
            let head = opt.get_or("head", "HEAD");
            let roots = opt.0.get("roots").map(String::as_str);
            let db_arg = opt.0.get("db").map(String::as_str);
            let explicit = [
                "base-scip-snapshot",
                "head-scip-snapshot",
                "base-graph-id",
                "head-graph-id",
                "base-source-prefix",
                "head-source-prefix",
            ]
            .iter()
            .any(|key| opt.0.contains_key(*key));
            let result = if explicit {
                impact_query::run_with_evidence(
                    repo,
                    base,
                    head,
                    roots,
                    db_arg,
                    impact_query::EvidenceByRevision {
                        base: Selection {
                            scip_snapshot: opt.0.get("base-scip-snapshot").map(String::as_str),
                            graph_id: opt.0.get("base-graph-id").map(String::as_str),
                            source_prefix: opt.0.get("base-source-prefix").map(String::as_str),
                        },
                        head: Selection {
                            scip_snapshot: opt.0.get("head-scip-snapshot").map(String::as_str),
                            graph_id: opt.0.get("head-graph-id").map(String::as_str),
                            source_prefix: opt.0.get("head-source-prefix").map(String::as_str),
                        },
                    },
                )?
            } else {
                let discovered = discovery::inspect(repo, head)?;
                let destination = analyze::checked_external_destination(
                    &discovered.root,
                    db_arg.map(Path::new).unwrap_or(&discovered.index_path),
                    "SQLite",
                )?;
                let (base_plan, head_plan) = if destination.is_file() {
                    let store = Db::open(&destination)?;
                    (
                        observed_relations::default_plan(&store, &discovered.root, base)?,
                        observed_relations::default_plan(&store, &discovered.root, head)?,
                    )
                } else {
                    (Plan::default(), Plan::default())
                };
                impact_query::run_with_plans(
                    repo, base, head, roots, db_arg, &base_plan, &head_plan,
                )?
            };
            if let Some(path) = opt.0.get("output") {
                let checkout = discovery::inspect(
                    Path::new(opt.get_or("path", ".")),
                    result.get("head_commit")?.str()?,
                )?;
                let destination = analyze::checked_external_destination(
                    &checkout.root,
                    Path::new(path),
                    "impact report",
                )?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|e| {
                        format!("create new impact report {}: {e}", destination.display())
                    })?;
                let write_result =
                    writeln!(file, "{}", result.canonical()).and_then(|_| file.sync_all());
                drop(file);
                if let Err(error) = write_result {
                    let _ = std::fs::remove_file(&destination);
                    return refuse(format!(
                        "write impact report {}: {error}",
                        destination.display()
                    ));
                }
            }
            if opt.get_or("json", "false") == "true" {
                Ok(json(result))
            } else {
                let mut human = impact_query::human(&result)?;
                if let Some(path) = opt.0.get("output") {
                    human.push_str(&format!("\nSaved replayable report: {path:?}"));
                }
                Ok(Response::Message(human))
            }
        }
        [top] if top == "changes" => Ok(json(change_inventory::inventory(
            Path::new(opt.get_or("path", ".")),
            opt.get("base"),
            opt.get_or("head", "HEAD"),
        )?)),
        [top] if top == "doctor" => {
            if opt.get_or("json", "false") == "true" {
                Ok(json(doctor::report()))
            } else {
                Ok(Response::Message(doctor::format_human(&doctor::report())?))
            }
        }
        [top] if top == "index" => {
            let roots = opt
                .get_or("roots", "src,tests")
                .split(',')
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>();
            if roots.is_empty() {
                return refuse("at least one module root is required");
            }
            let repo = opt.path("repo");
            let db = Db::open(opt.path("db"))?;
            let rev = opt.get_or("rev", "HEAD");
            let id = index::index_revision_with_roots(
                &db,
                repo,
                opt.get("name"),
                rev,
                "local-checkout",
                &roots,
            )?;
            let commit = git::rev(repo, rev)?;
            let tree = index::reconstruct_tree(&db, id, &commit)?;
            Ok(json(excession::json::obj(&[
                ("name", excession::json::Value::string(opt.get("name"))),
                ("source_commit", excession::json::Value::string(commit)),
                ("source_tree", excession::json::Value::string(tree)),
                ("status", excession::json::Value::string("GIT_TREE_INDEXED")),
            ])))
        }
        [top, sub] if top == "security" => {
            if sub == "scan" {
                return Ok(json(scan::run(opt)?));
            }
            if !matches!(sub.as_str(), "ingest" | "audit") {
                return refuse(format!("unknown security command {sub}"));
            }

            let db = Db::open(opt.path("db"))?;
            Ok(json(security_ingest::execute(
                opt.path("repo"),
                opt.path("report"),
                &db,
                opt.get("engine"),
                sub == "audit",
            )?))
        }
        [top, sub] if top == "refactor" && sub == "extract-shared-module" => Ok(json(
            refactor_codegen::extract_shared_module(&refactor_codegen::Request {
                repo: opt.path("repo"),
                revision: opt.get("rev"),
                index: opt.path("index"),
                report: opt.path("report"),
                candidate: opt.get("candidate"),
                source: opt.get("source"),
                destination: opt.get("destination"),
                symbols: opt.get("symbols"),
                ast_grep: Path::new(opt.get_or("ast-grep", "ast-grep")),
                source_prefix: opt.0.get("source-prefix").map(String::as_str),
                output: opt.0.get("output").map(Path::new),
            })?,
        )),
        [top, sub] if top == "refactor" && sub == "rust-assist" => {
            let coordinate = |key: &str| -> Result<usize> {
                opt.get(key)
                    .parse::<usize>()
                    .map_err(|_| format!("invalid LSP {key} coordinate"))
            };
            Ok(json(refactor_rust_analyzer::execute(
                &refactor_rust_analyzer::Request {
                    repo: opt.path("repo"),
                    revision: opt.get("rev"),
                    index: opt.path("index"),
                    report: opt.path("report"),
                    candidate: opt.get("candidate"),
                    source: opt.get("source"),
                    source_prefix: opt.0.get("source-prefix").map(String::as_str),
                    selection: [
                        coordinate("start-line")?,
                        coordinate("start-character")?,
                        coordinate("end-line")?,
                        coordinate("end-character")?,
                    ],
                    analyzer: Path::new(opt.get_or("rust-analyzer", "rust-analyzer")),
                    title: opt.0.get("action").map(String::as_str),
                    output: opt.0.get("output").map(Path::new),
                },
            )?))
        }
        [top, sub] if top == "refactor" && sub == "rust-repair" => Ok(json(
            refactor_rust_repair::repair(&refactor_rust_repair::Request {
                repo: opt.path("repo"),
                source_repo: opt.path("source-repo"),
                base: opt.get("base"),
                plan: opt.path("plan"),
                index: opt.path("index"),
                report: opt.path("report"),
                diagnostics: opt.path("diagnostics"),
                source_prefix: opt.0.get("source-prefix").map(String::as_str),
                analyzer: Path::new(opt.get_or("rust-analyzer", "rust-analyzer")),
                ast_grep: Path::new(opt.get_or("ast-grep", "ast-grep")),
                output: opt.0.get("output").map(Path::new),
            })?,
        )),
        [top, sub] if top == "refactor" && sub == "scip-ingest" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(scip_evidence::import(
                opt.path("repo"),
                opt.get("rev"),
                opt.path("index"),
                opt.0.get("source-prefix").map(String::as_str),
                &db,
            )?))
        }
        [top, sub] if top == "refactor" && sub == "scip-store-audit" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(scip_evidence::audit_source(
                opt.path("repo"),
                opt.get("rev"),
                opt.0.get("source-prefix").map(String::as_str),
                &db,
                opt.get("snapshot"),
            )?))
        }
        [top, sub]
            if top == "refactor" && matches!(sub.as_str(), "scip-suggest" | "scip-audit") =>
        {
            Ok(json(if sub == "scip-audit" {
                scip_refactor::audit_with_prefix(
                    opt.path("repo"),
                    opt.get("rev"),
                    opt.path("index"),
                    opt.path("report"),
                    opt.0.get("source-prefix").map(String::as_str),
                )?
            } else {
                scip_refactor::suggest_with_prefix(
                    opt.path("repo"),
                    opt.get("rev"),
                    opt.path("index"),
                    opt.0.get("source-prefix").map(String::as_str),
                )?
            }))
        }
        [top, sub] if top == "refactor" => {
            if !matches!(sub.as_str(), "suggest" | "audit") {
                return refuse(format!("unknown refactor command {sub}"));
            }

            let db = Db::open(opt.path("db"))?;
            Ok(json(if sub == "audit" {
                refactor::audit(&db, opt.get("name"), opt.get("rev"), opt.path("report"))?
            } else {
                refactor::suggest(&db, opt.get("name"), opt.get("rev"))?
            }))
        }
        [top, sub] if top == "benchmark" => match sub.as_str() {
            "review-prepare" => {
                let repetition = opt
                    .get("repetition")
                    .parse::<u64>()
                    .map_err(|_| "invalid --repetition".to_owned())?;
                let assignment = benchmark::historical_review::prepare(
                    opt.path("study"),
                    opt.get("case"),
                    opt.get("condition"),
                    repetition,
                    opt.path("out"),
                )?;
                Ok(Response::Message(format!(
                    "PREPARED {}",
                    assignment.display()
                )))
            }
            "review-record" => {
                let parse_tokens = |key: &str| -> Result<Option<u64>> {
                    opt.0
                        .get(key)
                        .map(|v| v.parse::<u64>().map_err(|_| format!("invalid --{key}")))
                        .transpose()
                };
                benchmark::historical_review::record(
                    opt.path("assignment"),
                    opt.path("transcript"),
                    opt.get("decision"),
                    opt.path("out"),
                    parse_tokens("input-tokens")?,
                    parse_tokens("output-tokens")?,
                )?;
                Ok(Response::Message(format!("RECORDED {}", opt.get("out"))))
            }
            "review-evaluate" => {
                benchmark::historical_review::evaluate(
                    opt.path("study"),
                    opt.path("attempts"),
                    opt.path("out"),
                )?;
                Ok(Response::Message(format!("EVALUATED {}", opt.get("out"))))
            }
            "validate" => {
                benchmark::validate(opt.path("report"))?;
                Ok(Response::Message(format!("VALID {}", opt.get("report"))))
            }
            "report" => {
                let output = match opt.get("kind") {
                    "relational" => {
                        if opt.0.contains_key("input") {
                            return refuse("--input is only valid for native calibration");
                        }
                        benchmark::build_relational(opt.path("directory"))?
                    }
                    "native" => {
                        if !opt.0.contains_key("input") {
                            return refuse("native calibration requires --input");
                        }
                        benchmark::build_native(opt.path("directory"), opt.get("input"))?
                    }
                    other => return refuse(format!("unknown benchmark report kind {other}")),
                };
                Ok(Response::Message(format!("VALID {}", output.display())))
            }
            "run-relational" => {
                let parse_sizes = |key: &str, default: &str| -> Result<Vec<usize>> {
                    opt.get_or(key, default)
                        .split(',')
                        .map(|value| {
                            value
                                .parse::<usize>()
                                .map_err(|_| format!("invalid --{key} element: {value}"))
                        })
                        .collect()
                };
                let revisions = opt
                    .get_or("revisions", "3")
                    .parse::<usize>()
                    .map_err(|_| "invalid --revisions".to_owned())?;
                let repetitions = opt
                    .get_or("repetitions", "2")
                    .parse::<usize>()
                    .map_err(|_| "invalid --repetitions".to_owned())?;
                let config = benchmark::relational::Config {
                    out: opt.path("out").to_path_buf(),
                    sizes: parse_sizes("sizes", "8,24")?,
                    fanouts: parse_sizes("fanouts", "1,3")?,
                    revisions,
                    repetitions,
                };
                let binary = std::env::current_exe()
                    .map_err(|e| format!("resolve benchmark executable: {e}"))?;
                let report = benchmark::relational::run(&config, &binary)?;
                Ok(Response::Message(format!("VALID {}", report.display())))
            }
            _ => refuse(format!("unknown benchmark command {sub}")),
        },
        [top, sub] if top == "reconstruct" && sub == "evaluate" => {
            let db = Db::open(opt.path("db"))?;
            let trusted = match opt.get("trusted-local") {
                "true" => true,
                "false" => false,
                _ => return refuse("--trusted-local requires true or false"),
            };
            Ok(json(excession::evidence::reconstruction::evaluate(
                &db,
                opt.path("target"),
                opt.path("candidate"),
                opt.get_or("compiler", "cc"),
                opt.get_or("objcopy", "objcopy"),
                trusted,
            )?))
        }
        [top, sub] if top == "reconstruct" && sub == "auto" => {
            let db = Db::open(opt.path("db"))?;
            let trusted = match opt.get("trusted-local") {
                "true" => true,
                "false" => false,
                _ => return refuse("--trusted-local requires true or false"),
            };
            let budget = opt
                .get_or("max-candidates", "64")
                .parse::<usize>()
                .map_err(|_| "invalid maximum reconstruction candidates")?;
            let seconds = opt
                .get_or("time-budget-seconds", "30")
                .parse::<u64>()
                .map_err(|_| "invalid reconstruction time budget")?;
            Ok(json(excession::evidence::reconstruction::search(
                &db,
                &excession::evidence::reconstruction::SearchOptions {
                    target: opt.path("target"),
                    output: opt.path("out"),
                    compiler: opt.get_or("compiler", "cc"),
                    objcopy: opt.get_or("objcopy", "objcopy"),
                    objdump: opt.get_or("objdump", "objdump"),
                    nm: opt.get_or("nm", "nm"),
                    budget,
                    seconds,
                    trusted_local: trusted,
                },
            )?))
        }
        [top, sub] if top == "reconstruct" && sub == "searches" => {
            let db = Db::open(opt.path("db"))?;
            let limit = opt
                .get_or("limit", "16")
                .parse::<usize>()
                .map_err(|_| "invalid reconstruction search history limit")?;
            Ok(json(excession::evidence::reconstruction::searches(
                &db,
                opt.path("target"),
                limit,
            )?))
        }
        [top, sub] if top == "reconstruct" && sub == "history" => {
            let db = Db::open(opt.path("db"))?;
            let limit = opt
                .get_or("limit", "16")
                .parse::<usize>()
                .map_err(|_| "invalid reconstruction history limit")?;
            Ok(json(excession::evidence::reconstruction::history(
                &db,
                opt.path("target"),
                limit,
            )?))
        }
        [top, sub] if top == "evidence" && sub == "ghidra-import" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::ghidra::import(
                &db,
                opt.path("artifact"),
                opt.path("report"),
                opt.path("native"),
            )?))
        }
        [top, sub] if top == "evidence" && sub == "ghidra-source-bind" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::ghidra::bind_source(
                &db,
                opt.path("repo"),
                opt.get("rev"),
                opt.get("source"),
                opt.path("candidate"),
                opt.get("symbol"),
            )?))
        }
        [top, sub] if top == "inspect" && sub == "ghidra-candidates" => {
            let db = Db::open(opt.path("db"))?;
            let limit = opt
                .get_or("limit", "16")
                .parse::<usize>()
                .map_err(|_| "invalid Ghidra candidate limit")?;
            Ok(json(excession::evidence::ghidra::candidates(
                &db,
                opt.path("artifact"),
                opt.get("entry"),
                limit,
            )?))
        }
        [top, sub] if top == "evidence" && sub == "rea-import" => {
            let db = Db::open(opt.path("db"))?;
            let build = rea_build_link(opt)?;
            Ok(json(excession::evidence::rea::import(
                &db,
                &excession::evidence::rea::ImportSpec {
                    artifact: opt.path("artifact"),
                    bundle: opt.path("bundle"),
                    build,
                },
            )?))
        }
        [top, sub] if top == "inspect" && sub == "artifact-evidence" => {
            let db = Db::open(opt.path("db"))?;
            let link = rea_build_link(opt)?;
            let limit = opt
                .get_or("limit", "16")
                .parse::<usize>()
                .map_err(|_| "invalid artifact evidence limit")?;
            Ok(json(excession::evidence::artifact::dossier(
                &db,
                opt.path("artifact"),
                link.as_ref(),
                limit,
            )?))
        }
        [top, sub] if top == "inspect" && sub == "rea-evidence" => {
            let db = Db::open(opt.path("db"))?;
            let link = rea_build_link(opt)?;
            Ok(json(excession::evidence::rea::audit(
                &db,
                opt.path("artifact"),
                opt.get("id"),
                link.as_ref(),
            )?))
        }
        [top, sub] if top == "inspect" && sub == "rea-record" => {
            let db = Db::open(opt.path("db"))?;
            let link = rea_build_link(opt)?;
            Ok(json(excession::evidence::rea::inspect_record(
                &db,
                opt.path("artifact"),
                opt.get("id"),
                opt.get("record"),
                link.as_ref(),
            )?))
        }
        [top, sub] if top == "evidence" && sub == "build-record" => {
            let db = Db::open(opt.path("db"))?;
            let sources = opt.get("sources").split(',').collect::<Vec<_>>();
            Ok(json(excession::evidence::build::record(
                &db,
                &excession::evidence::build::BuildSpec {
                    repo: opt.path("repo"),
                    revision: opt.get("rev"),
                    artifact: opt.path("artifact"),
                    inputs: &sources,
                    toolchain: opt.get("toolchain"),
                    configuration: opt.get("configuration"),
                    kind: opt.get("representation"),
                },
            )?))
        }
        [top, sub] if top == "inspect" && sub == "build-provenance" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::build::audit(
                &db,
                opt.path("repo"),
                opt.get("rev"),
                opt.path("artifact"),
                opt.get("snapshot"),
            )?))
        }
        [top, sub] if top == "evidence" && sub == "unknown-append" => {
            let expected = opt
                .get("expected")
                .parse::<usize>()
                .map_err(|_| "unknown expected revision must be an integer")?;
            let action = excession::evidence::unknowns::Action::from_name(opt.get("action"))?;
            let support = opt
                .0
                .get("support")
                .map(|s| s.split(',').collect::<Vec<_>>())
                .unwrap_or_default();
            let counter = opt
                .0
                .get("counter")
                .map(|s| s.split(',').collect::<Vec<_>>())
                .unwrap_or_default();
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::unknowns::append(
                &db,
                &excession::evidence::unknowns::Event {
                    snapshot_id: opt.get("snapshot"),
                    question: opt.get("question"),
                    expected_revision: expected,
                    action,
                    support_receipts: &support,
                    counterevidence_receipts: &counter,
                    resolution_receipt: opt.0.get("resolution").map(String::as_str),
                },
            )?))
        }
        [top, sub] if top == "inspect" && sub == "unknown-history" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::unknowns::audit(
                &db,
                opt.get("id"),
            )?))
        }
        [top, sub] if top == "inspect" && sub == "verify-observation" => {
            let db = Db::open(opt.path("db"))?;
            Ok(json(excession::evidence::claims::verify_scip_reference(
                &db,
                opt.path("repo"),
                opt.get("rev"),
                opt.get("snapshot"),
                opt.get("observation"),
                opt.get("source"),
                opt.get("predicate"),
            )?))
        }
        [top, sub] if top == "inspect" && sub == "producer-candidates" => {
            let discovered = discovery::inspect(opt.path("repo"), opt.get_or("rev", "HEAD"))?;
            let destination =
                analyze::checked_external_destination(&discovered.root, opt.path("db"), "SQLite")?;
            if !destination.is_file() {
                return refuse("producer discovery requires an existing SQLite evidence database");
            }
            let db = Db::open(&destination)?;
            Ok(json(observed_relations::discover(
                &db,
                &discovered.root,
                &discovered.commit,
            )?))
        }
        [top, sub] if top == "inspect" && sub == "scip-evidence" => {
            let db = Db::open(opt.path("db"))?;
            let focus = opt.0.get("focus").map(String::as_str);
            let after = opt.0.get("after").map(String::as_str);
            let source_prefix = opt.0.get("source-prefix").map(String::as_str);
            let source = match (opt.0.get("repo"), opt.0.get("rev")) {
                (None, None) if source_prefix.is_none() => None,
                (Some(repo), Some(rev)) => Some((repo.as_str(), rev.as_str())),
                _ => return refuse("SCIP source authentication requires both --repo and --rev"),
            };
            let result = match source {
                Some((repo, rev)) => scip_evidence::explain_authenticated(
                    &db,
                    Path::new(repo),
                    rev,
                    source_prefix,
                    opt.get("snapshot"),
                    focus,
                    after,
                )?,
                None => scip_evidence::explain_page(&db, opt.get("snapshot"), focus, after)?,
            };
            Ok(json(result))
        }
        [top, sub] if top == "inspect" && sub == "evidence" => Ok(json(evidence_query::query(
            opt.path("repo"),
            opt.get_or("rev", "HEAD"),
            opt.get("focus"),
            opt.path("syntax-report"),
            opt.path("resolution-report"),
            opt.0.get("declaration").map(|s| Path::new(s.as_str())),
            opt.0.get("runtime-report").map(|s| Path::new(s.as_str())),
        )?)),
        [top, sub] if top == "inspect" => inspect_cmd(sub, opt),
        [top, sub, verb] if top == "view" && sub == "replay" => view_cmd(verb, opt, true),
        [top, verb] if top == "view" => view_cmd(verb, opt, false),
        [top] if top == "browse" => {
            let max = opt
                .get_or("max-bytes", "8192")
                .parse::<usize>()
                .map_err(|_| "--max-bytes must be 1..65536".to_owned())?;
            if !(1..=65536).contains(&max) {
                return refuse("--max-bytes must be 1..65536");
            }
            browse(opt.path("repo"), opt.path("manifest"), max)?;
            Ok(Response::Bytes(Vec::new()))
        }
        _ => refuse("unknown Excession command; see excession --help"),
    }
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match execute(&args).and_then(Response::emit) {
        Ok(()) => (),
        Err(msg) => {
            eprintln!("REFUSED: {msg}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| (*s).to_owned()).collect()
    }
    #[test]
    fn options_are_validated_once_by_clap() {
        assert!(
            execute(&strings(&[
                "index",
                "--repo",
                "a",
                "--db",
                "x",
                "--name",
                "n",
                "--db",
                "duplicate"
            ]))
            .is_err()
        );
        assert!(execute(&strings(&["index", "--reppo", "a"])).is_err());
        assert!(execute(&strings(&["index", "--repo", "--db", "x"])).is_err());
    }
    #[test]
    fn retired_research_subcommand_is_rejected() {
        assert!(execute(&strings(&["research"])).is_err());
        assert!(execute(&strings(&["research", "--help"])).is_err());
        assert!(execute(&strings(&["research", "compose"])).is_err());
    }
    #[test]
    fn command_help_and_failure_are_ordinary_results() {
        assert!(matches!(
            execute(&strings(&["--help"])),
            Ok(Response::Message(_))
        ));
        assert!(matches!(
            execute(&strings(&["view", "--help"])),
            Ok(Response::Message(_))
        ));
        assert!(execute(&strings(&["view", "unknown"])).is_err());
        assert!(execute(&strings(&["index", "--repo", "x"])).is_err());
    }
    #[test]
    fn browser_paths_cannot_escape_view_root() {
        assert_eq!(parse_browser_path("views/a", "../b").unwrap(), "views/b");
        assert!(parse_browser_path("", "..").is_err());
        assert!(parse_browser_path("views", "../../src").is_err());
        assert_eq!(
            parse_browser_path("views/a", "/views/c").unwrap(),
            "views/c"
        );
    }
    #[test]
    fn browser_escapes_control_characters() {
        assert_eq!(safe_display(b"hi\x1b[31m\n", 100), "hi\\u{001b}[31m\n");
        assert!(safe_display(b"abcdef", 3).contains("truncated"));
    }
}
