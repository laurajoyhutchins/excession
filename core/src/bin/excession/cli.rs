//! Declarative command grammar. Keep CLI parsing separate from Git/SQLite execution.
//! Command discovery and execution consume the same parsed ArgMatches.
use clap::{Arg, Command, builder::NonEmptyStringValueParser, error::ErrorKind};
use excession::Result;
use std::collections::BTreeMap;
use std::iter;
use std::path::Path;

fn leaf(name: &'static str, required: &[&'static str], optional: &[&'static str]) -> Command {
    let mut command = Command::new(name);
    for key in required {
        command = command.arg(
            Arg::new(*key)
                .long(*key)
                .required(true)
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        );
    }
    for key in optional {
        command = command.arg(
            Arg::new(*key)
                .long(*key)
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        );
    }
    command
}

fn group(name: &'static str, commands: Vec<Command>) -> Command {
    commands.into_iter().fold(
        Command::new(name).subcommand_required(true),
        Command::subcommand,
    )
}

pub(super) fn command() -> Command {
    let replay = group(
        "replay",
        vec![
            leaf("deps", &["db", "name", "rev", "seed", "manifest"], &[]),
            leaf("security", &["db", "repo", "run", "manifest"], &[]),
            leaf("compose", &["repo", "op", "left", "right", "output"], &[]),
            leaf(
                "select",
                &["repo", "manifest", "axis", "value", "output"],
                &[],
            ),
            leaf("mount", &["repo", "manifest", "namespace", "output"], &[]),
        ],
    )
    .about("Recompute and authenticate a saved view");
    let views = group(
        "view",
        vec![
            leaf("deps", &["db", "name", "rev", "seed"], &[]),
            leaf("profile-deps", &["db", "name", "rev", "seed"], &[]),
            leaf("graph", &["db", "name", "rev", "seed", "kind"], &[]),
            leaf("paths", &["db", "name", "rev", "seed", "role"], &[]),
            leaf("security", &["db", "repo", "run"], &[]),
            leaf("compose", &["repo", "op", "left", "right"], &[]),
            leaf("select", &["repo", "manifest", "axis", "value"], &[]),
            leaf("mount", &["repo", "manifest", "namespace"], &[]),
            leaf("ls", &["repo", "manifest"], &["path"]),
            leaf("stat", &["repo", "manifest", "path"], &[]),
            leaf("cat", &["repo", "manifest", "path"], &[]),
            leaf("materialize", &["repo", "manifest", "output"], &[]),
            leaf("audit", &["repo", "manifest", "output"], &[]),
            replay,
        ],
    )
    .about("Excession authenticated repository views");
    let syntax = group(
        "syntax",
        vec![
            leaf("scan", &["repo", "roots"], &["rev"]),
            leaf("audit", &["repo", "report"], &[]),
            leaf("preflight", &["report"], &[]),
            leaf("resolve", &["repo", "report"], &[]),
            leaf("resolve-audit", &["repo", "syntax-report", "report"], &[]),
            leaf("resolve-preflight", &["report"], &[]),
            leaf(
                "native-run",
                &[
                    "repo",
                    "syntax-report",
                    "resolution-report",
                    "entry",
                    "language",
                    "trusted-local",
                ],
                &["import-roots"],
            ),
            leaf("native-preflight", &["declaration"], &[]),
            leaf(
                "native-compare",
                &["repo", "syntax-report", "resolution-report", "declaration"],
                &[],
            ),
            leaf(
                "native-audit",
                &[
                    "repo",
                    "syntax-report",
                    "resolution-report",
                    "declaration",
                    "report",
                ],
                &[],
            ),
        ],
    )
    .about("Language syntax observations and evidence audits");
    let semantic = group(
        "semantic",
        vec![
            leaf("scan", &["repo", "roots"], &["rev"]),
            leaf("audit", &["repo", "roots", "report"], &[]),
            leaf("resolve", &["repo", "syntax-report"], &[]),
            leaf(
                "export",
                &["repo", "syntax-report", "resolution-report"],
                &[],
            ),
        ],
    )
    .about("Versioned semantic candidate observations and ODCS contracts");
    let security = group(
        "security",
        vec![
            leaf(
                "scan",
                &["repo", "report", "db", "engine", "config", "trusted-local"],
                &["scanner", "codeql-db", "timeout-seconds"],
            ),
            leaf("ingest", &["repo", "report", "db", "engine"], &[]),
            leaf("audit", &["repo", "report", "db", "engine"], &[]),
        ],
    )
    .about("Trusted-local scanning and evidence ingestion");
    let inspect = group(
        "inspect",
        vec![
            leaf("dependencies", &["db", "name", "rev"], &[]),
            leaf("manifest", &["db", "name", "rev"], &[]),
            leaf("impact", &["db", "name", "rev", "seed"], &[]),
            leaf("review", &["db", "name", "old", "new"], &[]),
            leaf(
                "evidence",
                &["repo", "focus", "syntax-report", "resolution-report"],
                &["rev", "declaration", "runtime-report"],
            ),
            leaf(
                "scip-evidence",
                &["db", "snapshot"],
                &["focus", "after", "repo", "rev", "source-prefix"],
            ),
            leaf("unknown-history", &["db", "id"], &[]),
            leaf(
                "build-provenance",
                &["db", "repo", "rev", "artifact", "snapshot"],
                &[],
            ),
            leaf(
                "artifact-evidence",
                &["db", "artifact"],
                &["repo", "rev", "build-snapshot", "limit"],
            ),
            leaf(
                "rea-evidence",
                &["db", "artifact", "id"],
                &["repo", "rev", "build-snapshot"],
            ),
            leaf(
                "rea-record",
                &["db", "artifact", "id", "record"],
                &["repo", "rev", "build-snapshot"],
            ),
            leaf(
                "verify-observation",
                &[
                    "db",
                    "repo",
                    "rev",
                    "snapshot",
                    "observation",
                    "source",
                    "predicate",
                ],
                &[],
            ),
        ],
    )
    .about("Query indexed Git relationships");
    let evidence = group(
        "evidence",
        vec![
            leaf(
                "unknown-append",
                &["db", "snapshot", "question", "expected", "action"],
                &["support", "counter", "resolution"],
            ),
            leaf(
                "build-record",
                &[
                    "db",
                    "repo",
                    "rev",
                    "artifact",
                    "sources",
                    "toolchain",
                    "configuration",
                    "representation",
                ],
                &[],
            ),
            leaf(
                "rea-import",
                &["db", "artifact", "bundle"],
                &["repo", "rev", "build-snapshot"],
            ),
        ],
    )
    .about("Persist native build and other source/artifact evidence");
    let benchmark = group(
        "benchmark",
        vec![
            leaf("validate", &["report"], &[]),
            leaf("report", &["kind", "directory"], &["input"]),
            leaf(
                "run-relational",
                &["out"],
                &["sizes", "fanouts", "revisions", "repetitions"],
            ),
        ],
    )
    .about("Validate and generate benchmark reports");
    let refactor =
        group(
            "refactor",
            vec![
            leaf("suggest", &["db", "name", "rev"], &[]),
            leaf("audit", &["db", "name", "rev", "report"], &[]),
            leaf(
                "scip-suggest",
                &["repo", "rev", "index"],
                &["source-prefix"],
            ),
            leaf(
                "scip-audit",
                &["repo", "rev", "index", "report"],
                &["source-prefix"],
            ),
            leaf(
                "scip-ingest",
                &["repo", "rev", "index", "db"],
                &["source-prefix"],
            ),
            leaf(
                "scip-store-audit",
                &["repo", "rev", "db", "snapshot"],
                &["source-prefix"],
            ),
            leaf(
                "extract-shared-module",
                &[
                    "repo",
                    "rev",
                    "index",
                    "report",
                    "candidate",
                    "source",
                    "destination",
                    "symbols",
                ],
                &["ast-grep", "source-prefix", "output"],
            )
            .about(
                "Preview or apply a bounded AST-backed Rust extraction to a new detached worktree",
            ),
            leaf(
                "rust-assist",
                &[
                    "repo",
                    "rev",
                    "index",
                    "report",
                    "candidate",
                    "source",
                    "start-line",
                    "start-character",
                    "end-line",
                    "end-character",
                ],
                &["rust-analyzer", "source-prefix", "action", "output"],
            )
            .about("Find or apply rust-analyzer refactoring code actions with Git/SCIP evidence"),
            leaf(
                "rust-repair",
                &[
                    "repo", "source-repo", "base", "plan", "index",
                    "report", "diagnostics",
                ],
                &["source-prefix", "rust-analyzer", "ast-grep", "output"],
            )
            .about(
                "Select an unambiguous native import repair from independent compiler diagnostics",
            ),
        ],
        )
        .about("Source-bound refactoring hypotheses and guarded recipes");
    let configured_graph = group(
        "graph",
        vec![
            leaf(
                "import",
                &["build", "target"],
                &[
                    "repo",
                    "rev",
                    "db",
                    "provider",
                    "observations",
                    "trusted-local",
                    "ar",
                ],
            ),
            leaf("ls", &[], &["db", "build"]),
            leaf("show", &[], &["db", "build", "id", "target"]),
            leaf(
                "deps",
                &["node"],
                &["db", "build", "id", "target", "direction", "depth"],
            ),
            leaf(
                "path",
                &["from", "to"],
                &["db", "build", "id", "target", "depth"],
            ),
            leaf(
                "kbuild",
                &["repo", "build", "archive", "db", "trusted-local"],
                &["rev", "ar"],
            ),
            leaf(
                "assemble",
                &["repo", "build", "target", "observations", "db"],
                &["rev"],
            ),
        ],
    )
    .about("Configuration-bound build and source evidence");
    let clang = || {
        leaf(
            "clang",
            &["repo", "source", "compile-db", "trusted-local"],
            &["rev", "compiler", "config", "object-prefix"],
        )
    };
    let analyze = Command::new("analyze")
        .about("Index supported source candidates or perform configured Clang analysis")
        .arg(
            Arg::new("path")
                .index(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("rev")
                .long("rev")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("roots")
                .long("roots")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("db")
                .long("db")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(clap::ArgAction::SetTrue),
        )
        .subcommand(clang());
    let kernel = group(
        "kernel",
        vec![
            leaf("scan", &["repo", "roots"], &["rev"]),
            leaf("inventory", &["repo"], &["rev", "size"]),
            leaf("partition", &["repo", "index"], &["rev", "size"]),
            leaf("audit", &["repo", "dir"], &["rev", "size"]),
            clang(),
        ],
    )
    .about("Bounded, Git-bound Linux source observations and coverage");
    let agent = group(
        "agent",
        vec![
            leaf(
                "context",
                &["repo", "rev", "focus"],
                &["task", "limit", "roots", "db"],
            ),
            leaf(
                "verify",
                &["repo", "base", "candidate"],
                &["limit", "roots", "db", "receipts"],
            ),
        ],
    )
    .about("Bounded, machine-readable context and verification for coding agents");
    Command::new("excession")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Excession | Git-authenticated repository exploration and security evidence")
        .after_help(
            "JSON is written to stdout, errors to stderr. Views are read-only; scanner\n\
             findings are unverified candidates. See subcommand --help for exact options.",
        )
        .subcommand_required(true)
        .subcommand(
            Command::new("discover")
                .about("Read-only Git revision and supported source-root discovery")
                .arg(
                    Arg::new("path")
                        .index(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("rev")
                        .long("rev")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                ),
        )
        .subcommand(
            Command::new("explain")
                .about("Recompute and explain a pinned source-bound impact claim")
                .arg(
                    Arg::new("path")
                        .index(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("report")
                        .long("report")
                        .required(true)
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("id")
                        .long("id")
                        .required(true)
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("db")
                        .long("db")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("roots")
                        .long("roots")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("audit")
                        .long("audit")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("impact")
                .about("Investigate bounded affected-source candidates between two Git revisions")
                .arg(
                    Arg::new("path")
                        .index(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("base")
                        .long("base")
                        .required(true)
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("head")
                        .long("head")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("roots")
                        .long("roots")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("db")
                        .long("db")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("output")
                        .long("output")
                        .help("Save a replayable JSON report to a new absolute path outside the repository")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("changes")
                .about("List exact tracked Git blob changes between two commits")
                .arg(
                    Arg::new("path")
                        .index(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("base")
                        .long("base")
                        .required(true)
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("head")
                        .long("head")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Report local toolchain and bundled storage capabilities")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            leaf("index", &["repo", "db", "name"], &["rev", "roots"])
                .about("Index a Git revision and dependency roots"),
        )
        .subcommand(syntax)
        .subcommand(semantic)
        .subcommand(views)
        .subcommand(security)
        .subcommand(evidence)
        .subcommand(group(
            "reconstruct",
            vec![
                leaf("evaluate", &["db", "target", "candidate", "trusted-local"], &["compiler", "objcopy"]),
                leaf("history", &["db", "target"], &["limit"]),
                leaf("auto", &["db", "target", "out", "trusted-local"], &["compiler", "objcopy", "objdump", "nm", "max-candidates", "time-budget-seconds"]),
                leaf("searches", &["db", "target"], &["limit"]),
            ],
        ).about("Trusted-local compiler feedback for C object-code reconstruction"))
        .subcommand(inspect)
        .subcommand(refactor)
        .subcommand(benchmark)
        .subcommand(configured_graph)
        .subcommand(analyze)
        .subcommand(kernel)
        .subcommand(agent)
        .subcommand(
            leaf("browse", &["repo", "manifest"], &["max-bytes"])
                .about("Read-only virtual source browser"),
        )
}

/// Clap is the sole authority for parsing, required values, help and subcommands.
pub(super) enum ParseOutcome {
    Help(String),
    Command(Parsed),
}

pub(super) struct Parsed {
    pub(super) path: Vec<String>,
    pub(super) options: Options,
}

pub(super) fn parse(args: &[String]) -> Result<ParseOutcome> {
    let mut cli = command();
    if args.is_empty() {
        return Ok(ParseOutcome::Help(
            cli.render_help().to_string().trim_end().to_owned(),
        ));
    }
    let argv = iter::once("excession").chain(args.iter().map(String::as_str));
    match cli.try_get_matches_from(argv) {
        Ok(matches) => {
            let mut path = Vec::new();
            let mut leaf = &matches;
            while let Some((name, submatches)) = leaf.subcommand() {
                path.push(name.to_owned());
                leaf = submatches;
            }
            let mut values = BTreeMap::new();
            for id in leaf.ids() {
                if let Ok(Some(value)) = leaf.try_get_one::<String>(id.as_str()) {
                    values.insert(id.as_str().to_owned(), value.clone());
                } else if matches!(leaf.try_get_one::<bool>(id.as_str()), Ok(Some(&true))) {
                    values.insert(id.as_str().to_owned(), "true".to_owned());
                }
            }
            Ok(ParseOutcome::Command(Parsed {
                path,
                options: Options(values),
            }))
        }
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            Ok(ParseOutcome::Help(err.to_string().trim_end().to_owned()))
        }
        Err(err) => Err(err.to_string()),
    }
}

#[derive(Debug)]
pub(super) struct Options(pub(super) BTreeMap<String, String>);

impl Options {
    pub(super) fn get(&self, key: &str) -> &str {
        self.0.get(key).expect("clap validated required option")
    }

    pub(super) fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.0.get(key).map(String::as_str).unwrap_or(default)
    }

    pub(super) fn path(&self, key: &str) -> &Path {
        Path::new(self.get(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn nested_help_is_generated_from_the_actual_command_grammar() {
        let ParseOutcome::Help(text) =
            parse(&strings(&["view", "replay", "deps", "--help"])).unwrap()
        else {
            panic!("help expected");
        };
        assert!(text.contains("--manifest"));
        assert!(text.contains("--seed"));
        assert!(!text.contains("--output"));
        let ParseOutcome::Help(syntax) = parse(&strings(&["syntax", "--help"])).unwrap() else {
            panic!("help expected");
        };
        assert!(syntax.contains("native-audit"));
    }

    #[test]
    fn agent_grammar_requires_pinned_focus_or_candidate() {
        assert!(
            parse(&strings(&[
                "agent", "context", "--repo", "x", "--rev", "HEAD"
            ]))
            .is_err()
        );
        assert!(
            parse(&strings(&[
                "agent", "verify", "--repo", "x", "--base", "HEAD~1"
            ]))
            .is_err()
        );
        assert!(
            parse(&strings(&[
                "agent",
                "context",
                "--repo",
                "x",
                "--rev",
                "HEAD",
                "--focus",
                "src/main.rs",
                "--limit",
                "8",
            ]))
            .is_ok()
        );
        assert!(
            parse(&strings(&[
                "agent",
                "verify",
                "--repo",
                "x",
                "--base",
                "HEAD~1",
                "--candidate",
                "HEAD",
                "--receipts",
                "/tmp/receipts.json",
            ]))
            .is_ok()
        );
    }

    #[test]
    fn rejects_unknown_subcommands_and_duplicate_or_empty_values() {
        assert!(parse(&strings(&["view", "not-a-command"])).is_err());
        assert!(
            parse(&strings(&[
                "index", "--repo", "x", "--db", "y", "--name", "n", "--db", "z"
            ]))
            .is_err()
        );
        assert!(
            parse(&strings(&[
                "index", "--repo", "", "--db", "y", "--name", "n"
            ]))
            .is_err()
        );
    }
}
