//! Excession: the only installable binary, backed by one Rust Git/SQLite authority.
//! Historical experiments run under `excession research` without a second CLI.
use excession::sqlite::{Db, Param};
use excession::{
    Result, benchmark, git, graph_baseline, index, native_boundary, native_depinfo, preflight, projection,
    refactor, refuse, scip_refactor, security_ingest, syntax, syntax_resolution, views,
};
use std::collections::BTreeMap;
use std::env;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

#[path = "excession/research.rs"]
mod research;
#[path = "excession/scan.rs"]
mod scan;

const HELP: &str = r#"Excession | Git-authenticated repository exploration and security evidence

Usage:
  excession index --repo REPO --db DB --name NAME [--rev HEAD] [--roots package,tests]
  excession syntax scan --repo REPO --roots src[,tests] [--rev HEAD]
  excession syntax audit --repo REPO --report SAVED_JSON
  excession syntax preflight --report SAVED_JSON
  excession syntax resolve --repo REPO --report SAVED_SYNTAX_JSON
  excession syntax resolve-audit --repo REPO --syntax-report SAVED_SYNTAX_JSON --report SAVED_RESOLUTION_JSON
  excession syntax resolve-preflight --report SAVED_RESOLUTION_JSON
  excession syntax native-preflight --declaration NATIVE_DECLARATION_JSON
  excession syntax cargo-depfile --repo REPO --package-root core --depfile CARGO_D --subject core/src/lib.rs --rustc RUSTC --rustc-version VERSION [--rev HEAD]
  excession syntax native-compare --repo REPO --syntax-report SYNTAX_JSON --resolution-report CANDIDATES_JSON --declaration NATIVE_DECLARATION_JSON
  excession syntax native-audit --repo REPO --syntax-report SYNTAX_JSON --resolution-report CANDIDATES_JSON --declaration NATIVE_DECLARATION_JSON --report BOUNDARY_JSON
  excession view deps --db DB --name NAME --rev REV --seed MODULE
  excession view paths --db DB --name NAME --rev REV --seed MODULE --role ROLE
  excession view graph --db DB --name NAME --rev REV --seed MODULE --kind KIND
  excession view profile-deps --db DB --name NAME --rev REV --seed MODULE
  excession view security --db DB --repo REPO --run RUN_ID
  excession view compose --repo REPO --op union|intersection|difference --left JSON --right JSON
  excession view select --repo REPO --manifest JSON --axis role|source-prefix|virtual-prefix --value TEXT
  excession view mount --repo REPO --manifest JSON --namespace NAME
  excession view ls --repo REPO --manifest JSON [--path VIRTUAL_DIR]
  excession view stat|cat --repo REPO --manifest JSON --path VIRTUAL_FILE
  excession view materialize|audit --repo REPO --manifest JSON --output NEW_DIR
  excession view replay deps|security|compose|select|mount [same options, plus --manifest or --output]
  excession browse --repo REPO --manifest JSON [--max-bytes 8192]
  excession security ingest|audit --repo REPO --report JSON --db DB --engine codeql|semgrep
  excession security scan --repo REPO --report NEW_JSON --db DB --engine codeql|semgrep
      --config LOCAL_RULES --trusted-local true [--scanner EXECUTABLE]
      [--codeql-db PREBUILT_DB] [--timeout-seconds 300]
  excession inspect dependencies|manifest --db DB --name NAME --rev REV
  excession inspect impact --db DB --name NAME --rev REV --seed MODULE
  excession inspect review --db DB --name NAME --old REV --new REV
  excession refactor suggest --db DB --name NAME --rev REV
  excession refactor audit --db DB --name NAME --rev REV --report SAVED_JSON
  excession refactor scip-suggest --repo REPO --rev REV --index DECODED_SCIP_JSON
  excession refactor scip-audit --repo REPO --rev REV --index DECODED_SCIP_JSON --report SAVED_JSON
  excession benchmark report --kind relational --directory RESULT_DIR
  excession benchmark report --kind native --directory RESULT_DIR --input RECEIPT.json
  excession benchmark validate --report SAVED_REPORT_JSON
  excession research COMMAND [historical positional arguments]

Views are read-only and are not executable source checkouts. Scanner findings remain
UNVERIFIED_STATIC_CANDIDATES. JSON is written to stdout, errors to stderr;
`view cat` writes exact Git blob bytes to stdout. Redirect view-plan JSON to a file
before using it as a manifest. `research` preserves pinned legacy experiments.

Use `excession --version` or `excession help`. No network access is required.
"#;

#[derive(Debug)]
struct Options(BTreeMap<String, String>);
impl Options {
    fn parse(argv: &[String], required: &[&str], optional: &[&str]) -> Result<Self> {
        let mut out = BTreeMap::new();
        let mut it = argv.iter();
        while let Some(token) = it.next() {
            let key = token.strip_prefix("--").ok_or_else(|| {
                format!("unexpected positional argument {token:?}; use named options")
            })?;
            if !required.contains(&key) && !optional.contains(&key) {
                return refuse(format!("unknown option {token}"));
            }
            let value = it
                .next()
                .ok_or_else(|| format!("missing value for {token}"))?;
            if value.is_empty() || value.starts_with("--") || value.contains('\0') {
                return refuse(format!("invalid value for {token}"));
            }
            if out.insert(key.to_owned(), value.to_owned()).is_some() {
                return refuse(format!("duplicate option {token}"));
            }
        }
        for key in required {
            if !out.contains_key(*key) {
                return refuse(format!("missing required option --{key}"));
            }
        }
        Ok(Self(out))
    }
    fn get(&self, key: &str) -> &str {
        self.0.get(key).expect("required option validated")
    }
    fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.0.get(key).map(String::as_str).unwrap_or(default)
    }
    fn path(&self, key: &str) -> &Path {
        Path::new(self.get(key))
    }
}

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

fn view_cmd(verb: &str, argv: &[String], replay: bool) -> Result<Response> {
    if (verb, replay) == ("deps", false) {
        let opt = Options::parse(argv, &["db", "name", "rev", "seed"], &[])?;
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
            let opt = Options::parse(argv, &["db", "name", "rev", "seed", "manifest"], &[])?;
            let db = Db::open(opt.path("db"))?;
            let plan =
                projection::dependencies(&db, opt.get("name"), opt.get("rev"), opt.get("seed"))?;
            projection::replay(&plan, opt.path("manifest"))?
        }
        ("profile-deps", false) => {
            let opt = Options::parse(argv, &["db", "name", "rev", "seed"], &[])?;
            let db = Db::open(opt.path("db"))?;
            projection::profile_dependencies(&db, opt.get("name"), opt.get("rev"), opt.get("seed"))?
        }
        ("graph", false) => {
            let opt = Options::parse(argv, &["db", "name", "rev", "seed", "kind"], &[])?;
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
            let opt = Options::parse(argv, &["db", "name", "rev", "seed", "role"], &[])?;
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
            let opt = Options::parse(
                argv,
                if replay {
                    &["db", "repo", "run", "manifest"]
                } else {
                    &["db", "repo", "run"]
                },
                &[],
            )?;
            let db = Db::open(opt.path("db"))?;
            let plan = projection::security(&db, opt.path("repo"), opt.get("run"))?;
            if replay {
                projection::replay(&plan, opt.path("manifest"))?
            } else {
                plan
            }
        }
        ("compose", false) | ("compose", true) => {
            let opt = Options::parse(
                argv,
                if replay {
                    &["repo", "op", "left", "right", "output"]
                } else {
                    &["repo", "op", "left", "right"]
                },
                &[],
            )?;
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
            let opt = Options::parse(
                argv,
                if replay {
                    &["repo", "manifest", "axis", "value", "output"]
                } else {
                    &["repo", "manifest", "axis", "value"]
                },
                &[],
            )?;
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
            let opt = Options::parse(
                argv,
                if replay {
                    &["repo", "manifest", "namespace", "output"]
                } else {
                    &["repo", "manifest", "namespace"]
                },
                &[],
            )?;
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
        ("ls", false) => {
            let opt = Options::parse(argv, &["repo", "manifest"], &["path"])?;
            projection::list(
                opt.path("repo"),
                opt.path("manifest"),
                opt.get_or("path", ""),
            )?
        }
        ("stat", false) => {
            let opt = Options::parse(argv, &["repo", "manifest", "path"], &[])?;
            projection::describe(opt.path("repo"), opt.path("manifest"), opt.get("path"))?
        }
        ("cat", false) => {
            let opt = Options::parse(argv, &["repo", "manifest", "path"], &[])?;
            return Ok(Response::Bytes(projection::open(
                opt.path("repo"),
                opt.path("manifest"),
                opt.get("path"),
            )?));
        }
        ("materialize", false) | ("audit", false) => {
            let opt = Options::parse(argv, &["repo", "manifest", "output"], &[])?;
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

fn inspect_cmd(verb: &str, argv: &[String]) -> Result<Response> {
    let (req, extra): (&[&str], &[&str]) = match verb {
        "dependencies" | "manifest" => (&["db", "name", "rev"], &[]),
        "impact" => (&["db", "name", "rev", "seed"], &[]),
        "review" => (&["db", "name", "old", "new"], &[]),
        _ => return refuse(format!("unknown inspection command {verb:?}")),
    };
    let opt = Options::parse(argv, req, extra)?;
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

fn execute(args: &[String]) -> Result<Response> {
    match args {
        [] => Ok(Response::Message(HELP.to_owned())),
        [a] if matches!(a.as_str(), "--help" | "-h" | "help") => {
            Ok(Response::Message(HELP.to_owned()))
        }
        [a] if a == "--version" => Ok(Response::Message(format!(
            "excession {}",
            env!("CARGO_PKG_VERSION")
        ))),
        [top, a]
            if a == "--help"
                && matches!(
                    top.as_str(),
                    "view" | "security" | "inspect" | "index" | "browse" | "syntax" | "refactor"
                ) =>
        {
            Ok(Response::Message(HELP.to_owned()))
        }
        [top, sub, a]
            if a == "--help"
                && matches!(
                    top.as_str(),
                    "view" | "security" | "inspect" | "syntax" | "refactor"
                )
                && !sub.is_empty() =>
        {
            Ok(Response::Message(HELP.to_owned()))
        }
        [top, tail @ ..] if top == "research" => {
            research::run(tail)?;
            Ok(Response::Bytes(Vec::new()))
        }
        [top, sub, tail @ ..] if top == "syntax" => match sub.as_str() {
            "scan" => {
                let opt = Options::parse(tail, &["repo", "roots"], &["rev"])?;
                let roots = opt.get("roots").split(',').collect::<Vec<_>>();
                Ok(json(syntax::scan(
                    opt.path("repo"),
                    opt.get_or("rev", "HEAD"),
                    &roots,
                )?))
            }
            "audit" => {
                let opt = Options::parse(tail, &["repo", "report"], &[])?;
                Ok(json(syntax::audit(opt.path("repo"), opt.path("report"))?))
            }
            "preflight" => {
                let opt = Options::parse(tail, &["report"], &[])?;
                Ok(json(preflight::syntax_file(opt.path("report"))?))
            }
            "resolve-preflight" => {
                let opt = Options::parse(tail, &["report"], &[])?;
                Ok(json(preflight::resolution_file(opt.path("report"))?))
            }
            "cargo-depfile" => {
                let opt = Options::parse(
                    tail,
                    &[
                        "repo",
                        "package-root",
                        "depfile",
                        "subject",
                        "rustc",
                        "rustc-version",
                    ],
                    &["rev"],
                )?;
                Ok(json(native_depinfo::declaration(
                    opt.path("repo"),
                    opt.get("package-root"),
                    opt.path("depfile"),
                    opt.get("subject"),
                    opt.path("rustc"),
                    opt.get("rustc-version"),
                    opt.get_or("rev", "HEAD"),
                )?))
            }
            "native-preflight" => {
                let opt = Options::parse(tail, &["declaration"], &[])?;
                Ok(json(native_boundary::preflight_file(
                    opt.path("declaration"),
                )?))
            }
            "native-compare" => {
                let opt = Options::parse(
                    tail,
                    &["repo", "syntax-report", "resolution-report", "declaration"],
                    &[],
                )?;
                Ok(json(native_boundary::compare(
                    opt.path("repo"),
                    opt.path("syntax-report"),
                    opt.path("resolution-report"),
                    opt.path("declaration"),
                )?))
            }
            "native-audit" => {
                let opt = Options::parse(
                    tail,
                    &[
                        "repo",
                        "syntax-report",
                        "resolution-report",
                        "declaration",
                        "report",
                    ],
                    &[],
                )?;
                Ok(json(native_boundary::audit(
                    opt.path("repo"),
                    opt.path("syntax-report"),
                    opt.path("resolution-report"),
                    opt.path("declaration"),
                    opt.path("report"),
                )?))
            }
            "resolve" => {
                let opt = Options::parse(tail, &["repo", "report"], &[])?;
                Ok(json(syntax_resolution::candidates(
                    opt.path("repo"),
                    opt.path("report"),
                )?))
            }
            "resolve-audit" => {
                let opt = Options::parse(tail, &["repo", "syntax-report", "report"], &[])?;
                Ok(json(syntax_resolution::audit(
                    opt.path("repo"),
                    opt.path("syntax-report"),
                    opt.path("report"),
                )?))
            }
            _ => refuse(format!("unknown syntax command {sub}")),
        },
        [top, tail @ ..] if top == "index" => {
            let opt = Options::parse(tail, &["repo", "db", "name"], &["rev", "roots"])?;
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
        [top, sub, rest @ ..] if top == "security" => {
            if sub == "scan" {
                let opt = Options::parse(
                    rest,
                    &["repo", "report", "db", "engine", "config", "trusted-local"],
                    &["scanner", "codeql-db", "timeout-seconds"],
                )?;
                return Ok(json(scan::run(&opt)?));
            }
            if !matches!(sub.as_str(), "ingest" | "audit") {
                return refuse(format!("unknown security command {sub}"));
            }
            let opt = Options::parse(rest, &["repo", "report", "db", "engine"], &[])?;
            let db = Db::open(opt.path("db"))?;
            Ok(json(security_ingest::execute(
                opt.path("repo"),
                opt.path("report"),
                &db,
                opt.get("engine"),
                sub == "audit",
            )?))
        }
        [top, sub, rest @ ..]
            if top == "refactor" && matches!(sub.as_str(), "scip-suggest" | "scip-audit") =>
        {
            let opt = Options::parse(
                rest,
                if sub == "scip-audit" {
                    &["repo", "rev", "index", "report"]
                } else {
                    &["repo", "rev", "index"]
                },
                &[],
            )?;
            Ok(json(if sub == "scip-audit" {
                scip_refactor::audit(
                    opt.path("repo"),
                    opt.get("rev"),
                    opt.path("index"),
                    opt.path("report"),
                )?
            } else {
                scip_refactor::suggest(opt.path("repo"), opt.get("rev"), opt.path("index"))?
            }))
        }
        [top, sub, rest @ ..] if top == "refactor" => {
            if !matches!(sub.as_str(), "suggest" | "audit") {
                return refuse(format!("unknown refactor command {sub}"));
            }
            let opt = Options::parse(
                rest,
                if sub == "audit" {
                    &["db", "name", "rev", "report"]
                } else {
                    &["db", "name", "rev"]
                },
                &[],
            )?;
            let db = Db::open(opt.path("db"))?;
            Ok(json(if sub == "audit" {
                refactor::audit(&db, opt.get("name"), opt.get("rev"), opt.path("report"))?
            } else {
                refactor::suggest(&db, opt.get("name"), opt.get("rev"))?
            }))
        }
        [top, sub, rest @ ..] if top == "benchmark" => match sub.as_str() {
            "validate" => {
                let opt = Options::parse(rest, &["report"], &[])?;
                benchmark::validate(opt.path("report"))?;
                Ok(Response::Message(format!("VALID {}", opt.get("report"))))
            }
            "report" => {
                let opt = Options::parse(rest, &["kind", "directory"], &["input"])?;
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
        [top, sub, rest @ ..] if top == "inspect" => inspect_cmd(sub, rest),
        [top, sub, rest @ ..] if top == "view" && sub == "replay" => {
            let Some((verb, tail)) = rest.split_first() else {
                return refuse("expected view replay operation");
            };
            view_cmd(verb, tail, true)
        }
        [top, verb, rest @ ..] if top == "view" => view_cmd(verb, rest, false),
        [top, rest @ ..] if top == "browse" => {
            let opt = Options::parse(rest, &["repo", "manifest"], &["max-bytes"])?;
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
    fn flags_are_strict() {
        assert!(Options::parse(&strings(&["--repo", "a", "--repo", "b"]), &["repo"], &[]).is_err());
        assert!(Options::parse(&strings(&["--reppo", "a"]), &["repo"], &[]).is_err());
        assert!(Options::parse(&strings(&["--repo", "--db", "x"]), &["repo"], &[]).is_err());
        assert!(Options::parse(&strings(&["--repo", "a", "surprise"]), &["repo"], &[]).is_err());
        assert_eq!(
            Options::parse(&strings(&["--repo", "a"]), &["repo"], &[])
                .unwrap()
                .get("repo"),
            "a"
        );
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
