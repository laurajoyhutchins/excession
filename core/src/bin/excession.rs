//! Excession: the only installable binary, backed by one Rust Git/SQLite authority.
//! All commands use one clap grammar; the same parsed options reach the native handlers.

use excession::analyze;
use excession::discovery;
use excession::doctor;
use excession::evidence_query;
use excession::semantic_contract;
use excession::sqlite::{Db, Param};
use excession::{
    Result, benchmark, compiler_clang, git, graph_assembly, graph_baseline, index, kbuild_graph,
    kernel, kernel_coverage, native_boundary, preflight, projection, refactor, refuse,
    scip_refactor, security_ingest, syntax, syntax_resolution, views,
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
    CanonicalJson(String),
    Message(String),
    Bytes(Vec<u8>),
}
impl Response {
    fn emit(self) -> Result<()> {
        let mut out = io::stdout().lock();
        match self {
            Self::Json(j) => writeln!(out, "{}", j.canonical()),
            Self::CanonicalJson(s) | Self::Message(s) => writeln!(out, "{s}"),
            Self::Bytes(bytes) => out.write_all(&bytes),
        }
        .map_err(|e| format!("stdout write failed: {e}"))
    }
}
fn json(v: excession::json::Value) -> Response {
    Response::Json(v)
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
        [top, sub] if top == "analyze" && sub == "clang" => clang_command(opt),
        [top] if top == "analyze" => {
            let (report, db_path) = analyze::run(
                Path::new(opt.get_or("path", ".")),
                opt.get_or("rev", "HEAD"),
                opt.0.get("roots").map(String::as_str),
                opt.0.get("db").map(String::as_str),
            )?;
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
            _ => refuse(format!("unknown benchmark command {sub}")),
        },
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
