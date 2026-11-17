//! Declarative command grammar. Keep CLI parsing separate from Git/SQLite execution.
//! Command discovery and execution consume the same parsed ArgMatches.
use clap::{Arg, ArgGroup, Command, builder::NonEmptyStringValueParser, error::ErrorKind};
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
    .about("Excession authenticated repository views")
    .hide(true);
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
    .about("Language syntax observations and evidence audits")
    .hide(true);
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
    .about("Versioned semantic candidate observations and ODCS contracts")
    .hide(true);
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
    .about("Trusted-local scanning and evidence ingestion")
    .hide(true);
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
            leaf("producer-candidates", &["repo", "db"], &["rev"]),
            leaf(
                "ghidra-candidates",
                &["db", "artifact", "entry"],
                &["limit"],
            ),
            leaf(
                "build-provenance",
                &["db", "repo", "rev", "artifact", "snapshot"],
                &[],
            ),
            leaf(
                "signed-artifact",
                &["db", "repo", "rev", "artifact", "snapshot"],
                &[],
            )
            .about("Reverify the original signed artifact bundle and Git source using GitHub CLI"),
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
    .about("Query indexed Git relationships")
    .hide(true);
    let evidence = group(
        "evidence",
        vec![
            leaf(
                "jvm-inputs-capture",
                &[
                    "repo",
                    "rev",
                    "source-prefix",
                    "classpath-file",
                    "provider-claim",
                ],
                &["release"],
            )
            .about("Hash an existing Maven/Gradle classpath export without executing a build"),
            leaf("native-import", &["repo", "rev", "report", "db"], &[])
                .about("Persist a Git-replayed Go/Clang producer report without executing it"),
            leaf(
                "native-store-audit",
                &["repo", "rev", "snapshot", "db"],
                &[],
            )
            .about("Replay stored native evidence against its pinned Git commit"),
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
                "signed-artifact",
                &["db", "repo", "rev", "artifact", "signed-repo",
                    "signer-workflow", "source-ref", "trusted-local"],
                &[],
            ).about("Cryptographically verify a GitHub-signed artifact claim and persist original signed evidence"),
            leaf(
                "ghidra-import",
                &["db", "artifact", "report", "native"],
                &[],
            ),
            leaf(
                "ghidra-source-bind",
                &["db", "repo", "rev", "source", "candidate", "symbol"],
                &[],
            ),
            leaf(
                "rea-import",
                &["db", "artifact", "bundle"],
                &["repo", "rev", "build-snapshot"],
            ),
        ],
    )
    .about("Persist native build and other source/artifact evidence")
    .hide(true);
    let benchmark = group(
        "benchmark",
        vec![
            leaf(
                "review-prepare",
                &["study", "case", "condition", "repetition", "out"],
                &[],
            ),
            leaf(
                "review-record",
                &["assignment", "transcript", "decision", "out"],
                &["input-tokens", "output-tokens"],
            ),
            leaf("review-evaluate", &["study", "attempts", "out"], &[]),
            leaf("validate", &["report"], &[]),
            leaf("report", &["kind", "directory"], &["input"]),
            leaf(
                "run-relational",
                &["out"],
                &["sizes", "fanouts", "revisions", "repetitions"],
            ),
        ],
    )
    .about("Validate and generate benchmark reports")
    .hide(true);
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
        .about("Source-bound refactoring hypotheses and guarded recipes")
        .hide(true);
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
    .about("Configuration-bound build and source evidence")
    .hide(true);
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
        .arg(
            Arg::new("scip-snapshot")
                .long("scip-snapshot")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("graph-id")
                .long("graph-id")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .arg(
            Arg::new("source-prefix")
                .long("source-prefix")
                .num_args(1)
                .value_parser(NonEmptyStringValueParser::new()),
        )
        .subcommand(clang())
        .subcommand(
            leaf(
                "native",
                &["language", "source"],
                &["repo", "rev", "compile-db", "compiler", "source-prefix", "classpath-manifest", "rust-analyzer", "scip"],
            )
            .about("Run a Git-bound Go, Java, C/C++ or Rust native tool")
            .arg(
                Arg::new("trusted-local")
                    .long("trusted-local")
                    .help("Execute local compiler tooling without a sandbox")
                    .num_args(0..=1)
                    .default_missing_value("true")
                    .value_parser(["true", "false"]),
            )
            .arg(
                Arg::new("json")
                    .long("json")
                    .help("Emit Git-bound native producer evidence")
                    .action(clap::ArgAction::SetTrue),
            )
            .after_help("Go:    excession analyze native --language go --source cmd/main.go --trusted-local\nClang: excession analyze native --language c --source src/main.c --compile-db build/compile_commands.json --trusted-local\nJava:  excession analyze native --language java --source src/main/java/pkg/Service.java --source-prefix src/main/java --trusted-local\nConfigured Java: add --classpath-manifest /tmp/resolved-inputs.json\nRust:  excession analyze native --language rust --source src/lib.rs --trusted-local\n       Optionally set --source-prefix, --rust-analyzer and --scip.\n\nGo selects compiler inputs; Clang observes one configured TU; javac resolves source references; rust-analyzer observes SCIP symbols. None alone proves build/runtime completeness."),
        )
        .subcommand(
            leaf("languages", &[], &["repo", "rev", "roots"])
                .about("Parse tracked source across languages, resolve conservative imports, report gaps")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                )
                .after_help("Examples:\n  excession analyze languages\n  excession analyze languages --roots src,tests --json\n\nRequires a clean checkout and preinstalled grammars. Missing grammars and unsupported resolvers are reported, never guessed."),
        )
        .subcommand(
            leaf(
                "rust",
                &[],
                &["repo", "rev", "source-prefix", "rust-analyzer", "scip"],
            )
            .about("Run rust-analyzer SCIP on a clean Git checkout")
            .arg(
                Arg::new("trusted-local")
                    .long("trusted-local")
                    .help("Permit running local Rust tooling; this is not sandboxed")
                    .num_args(0..=1)
                    .default_missing_value("true")
                    .value_parser(["true", "false"]),
            )
            .arg(
                Arg::new("json")
                    .long("json")
                    .help("Emit full versioned native SCIP evidence")
                    .action(clap::ArgAction::SetTrue),
            )
            .after_help("Examples:\n  excession analyze rust --trusted-local\n  excession analyze rust --repo . --trusted-local --json\n\nA sole tracked Cargo project is detected automatically. Use --source-prefix to choose among multiple projects."),
        );
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
    .about("Bounded, Git-bound Linux source observations and coverage")
    .hide(true);
    let tests = leaf(
        "tests",
        &[],
        &[
            "repo",
            "base",
            "candidate",
            "provider",
            "tool",
            "focus",
            "sandbox-image",
            "expected-snapshot",
            "timeout",
        ],
    )
    .visible_alias("test")
    .about("Plan and run native tests for committed or safely sandboxed uncommitted code")
    .arg(
        Arg::new("working-tree")
            .long("working-tree")
            .action(clap::ArgAction::SetTrue)
            .help("Explicitly snapshot the uncommitted working tree (also auto-detected)"),
    )
    .arg(
        Arg::new("doctor")
            .long("doctor")
            .action(clap::ArgAction::SetTrue)
            .help("Inspect rootless sandbox prerequisites without requiring a Git checkout"),
    )
    .arg(
        Arg::new("run")
            .long("run")
            .action(clap::ArgAction::SetTrue)
            .help("Execute native tests (uncommitted work runs only in the isolated sandbox)"),
    )
    .arg(
        Arg::new("verify")
            .long("verify")
            .action(clap::ArgAction::SetTrue)
            .help("Carry native test receipts into agent verification (requires --run)"),
    )
    .arg(
        Arg::new("json")
            .long("json")
            .action(clap::ArgAction::SetTrue)
            .help("Emit canonical JSON for an agent or script"),
    );
    let investigation = group(
        "investigation",
        vec![
            leaf("start", &["repo", "db", "rev", "question"], &[])
                .about("Start a persistent question-scoped investigation at a Git commit"),
            leaf(
                "claim",
                &[
                    "repo",
                    "db",
                    "id",
                    "expected-revision",
                    "path",
                    "statement",
                    "assumption",
                    "assessment",
                ],
                &[],
            )
            .about("Record a Git-bound hypothesis or reported source review"),
            leaf(
                "plan",
                &[
                    "repo",
                    "db",
                    "id",
                    "expected-revision",
                    "claim-event",
                    "obligation",
                ],
                &[],
            )
            .about("Attach a behavior-preservation obligation to an earlier claim"),
            leaf(
                "verify",
                &[
                    "repo",
                    "db",
                    "id",
                    "expected-revision",
                    "plan-event",
                    "candidate",
                ],
                &["limit", "roots", "evidence-db", "receipts"],
            )
            .about("Run existing agent verification and retain its unresolved obligations"),
            leaf(
                "explore",
                &["repo", "db", "id", "expected-revision"],
                &["limit", "index-segments", "roots", "evidence-db"],
            )
            .about("Persist Git-pinned candidate source and an incomplete reading plan"),
            leaf(
                "question",
                &[
                    "repo",
                    "db",
                    "id",
                    "expected-revision",
                    "question",
                    "reason",
                ],
                &[],
            )
            .about("Record an unresolved investigation question"),
            leaf(
                "synthesize",
                &[
                    "repo",
                    "db",
                    "id",
                    "expected-revision",
                    "narrative",
                    "claim-events",
                ],
                &[],
            )
            .about("Record an attributed explanation linked to existing source claims"),
            leaf(
                "refresh",
                &["repo", "db", "id", "expected-revision", "candidate"],
                &[],
            )
            .about("Reassess exact claim source identity at a candidate revision"),
            leaf("explain", &["repo", "db", "id"], &[])
                .about("Compose a source-linked explanation and expose unresolved obligations"),
            leaf("show", &["repo", "db", "id"], &[])
                .about("Audit and replay an entire persistent investigation"),
        ],
    )
    .about("Persist Git-authenticated claims, preservation plans and verification evidence");
    let agent = group(
        "agent",
        vec![
            leaf("init", &[], &["repo", "client"])
                .about("Plan or create project-level MCP configuration without replacing existing files")
                .arg(Arg::new("write").long("write")
                    .help("Explicitly create missing config files; default is read-only plan")
                    .action(clap::ArgAction::SetTrue)),
            leaf(
                "locate",
                &["repo", "task"],
                &["rev", "limit", "roots", "db", "index-segments", "budget-file", "max-file-bytes", "max-segment-bytes", "max-segment-files"],
            )
            .about("Find ranked Git-pinned source candidates for a natural-language task")
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(clap::ArgAction::SetTrue)
                    .help("Emit canonical JSON (also the default for agent locate)"),
            ),
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
            leaf(
                "select-tests",
                &["repo", "base", "candidate", "provider", "trusted-local"],
                &["tool", "focus"],
            )
            .about("Delegate selection to installed Jest or pytest-testmon on a trusted checkout"),
        ],
    )
    .about("Bounded, machine-readable context and verification for coding agents");
    Command::new("excession")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Excession: Git-grounded analysis with evidence and explicit unknowns")
        .after_help("Start here:\n  excession analyze .\n  excession impact .             # HEAD~1..HEAD\n  excession tests                # plan only\n  excession doctor\n\nRun 'excession commands' for the full catalog.\nNative analysis and test execution are always opt-in.")
        .subcommand_required(true)
        .subcommand(analyze)
        .subcommand(group(
            "check",
            vec![
                leaf(
                    "run",
                    &["repo", "language", "config", "db", "trusted-local"],
                    &["rev", "source-prefix", "tool"],
                ).about("Run a bounded local Cargo, TypeScript or mypy check and store its Git-bound receipt"),
                leaf(
                    "audit",
                    &["repo", "rev", "db", "snapshot"],
                    &[],
                ).about("Replay a stored compiler-check receipt against its original Git configuration"),
                leaf(
                    "matrix",
                    &["repo", "manifest", "db", "trusted-local"],
                    &["rev", "rust-tool", "typescript-tool", "python-tool"],
                ).about("Run all checks in one Git-declared compiler matrix and persist the aggregate"),
                leaf(
                    "matrix-audit",
                    &["repo", "rev", "db", "snapshot"],
                    &[],
                ).about("Replay matrix and all child compiler receipts against original Git objects"),
            ],
        ).about("Explicit local compiler/typechecker receipts; not artifact build attestation"))
         .subcommand(Command::new("commands").about("List all commands, including specialist tools"))
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
                .about("Replay and explain distinct sources or a batch of source witnesses")
                .group(ArgGroup::new("explanation-target")
                    .args(["source", "all-sources"])
                    .required(true)
                    .multiple(false))
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
                    Arg::new("source")
                        .long("source")
                        .help("Git-relative path of a distinct impacted source")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("source-commit")
                        .long("source-commit")
                        .help("Disambiguate a dependent source present in both revisions")
                        .requires("source")
                        .conflicts_with("all-sources")
                        .num_args(1)
                        .value_parser(NonEmptyStringValueParser::new()),
                )
                .arg(
                    Arg::new("all-sources")
                        .long("all-sources")
                        .help("Batch reconstruct distinct-source witnesses with one pinned replay")
                        .action(clap::ArgAction::SetTrue),
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
                .arg(Arg::new("base-scip-snapshot").long("base-scip-snapshot").num_args(1).value_parser(NonEmptyStringValueParser::new()))
                .arg(Arg::new("head-scip-snapshot").long("head-scip-snapshot").num_args(1).value_parser(NonEmptyStringValueParser::new()))
                .arg(Arg::new("base-graph-id").long("base-graph-id").num_args(1).value_parser(NonEmptyStringValueParser::new()))
                .arg(Arg::new("head-graph-id").long("head-graph-id").num_args(1).value_parser(NonEmptyStringValueParser::new()))
                .arg(Arg::new("base-source-prefix").long("base-source-prefix").num_args(1).value_parser(NonEmptyStringValueParser::new()))
                .arg(Arg::new("head-source-prefix").long("head-source-prefix").num_args(1).value_parser(NonEmptyStringValueParser::new()))

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
                .about("Index a Git revision and dependency roots").hide(true),
        )
        .subcommand(syntax)
        .subcommand(semantic)
        .subcommand(views)
        .subcommand(security)
        .subcommand(evidence)
        .subcommand(group(
            "history",
            vec![
                leaf("plan", &["repo"], &["rev", "base", "max-commits"]),
                leaf("audit", &["repo", "plan"], &[]),
                leaf("refs", &["repo"], &["max-refs"]),
                leaf("verify-target", &["repo", "target", "replacement"], &[]),
                leaf("preview-dag", &["repo", "output"], &["group", "max-commits"]),
            ],
        ).about("Read-only Git history consolidation plans and exact source-DAG replay").hide(true))
        .subcommand(group(
            "reconstruct",
            vec![
                leaf("evaluate", &["db", "target", "candidate", "trusted-local"], &["compiler", "objcopy"]),
                leaf("history", &["db", "target"], &["limit"]),
                leaf("auto", &["db", "target", "out", "trusted-local"], &["compiler", "objcopy", "objdump", "nm", "max-candidates", "time-budget-seconds"]),
                leaf("searches", &["db", "target"], &["limit"]),
                leaf("git", &["repo", "plan"], &[]),
                leaf("git-synthesize", &["repo", "seed", "output"], &["max-candidates", "feedback"]),
                leaf("git-evidence", &["repo", "plan", "db", "evidence"], &[]),
                leaf("git-oracles", &["repo", "plan", "db", "evidence", "checks", "trusted-local"], &[]),
            ],
        ).about("Read-only Git reconstruction and trusted-local C compiler feedback").hide(true))
        .subcommand(inspect)
        .subcommand(refactor)
        .subcommand(group(
            "improve",
            vec![
                leaf(
                    "plan",
                    &["rev"],
                    &["producer", "repo", "index", "source-prefix", "db", "name"],
                ),
                leaf(
                    "replay",
                    &["repo", "rev", "plan", "edits"],
                    &["output"],
                ).about("Preview or replay pinned language-native edits in a detached worktree"),
                leaf(
                    "trial",
                    &[
                        "repo", "rev", "index", "report", "plan",
                        "candidate", "trusted-local",
                    ],
                    &["producer", "source-prefix", "ast-grep", "symbol", "output"],
                ),
                leaf(
                    "evaluate",
                    &["base", "candidate", "plan"],
                    &[
                        "producer", "base-repo", "candidate-repo",
                        "base-index", "candidate-index", "source-prefix",
                        "repo", "db", "name",
                    ],
                ),
            ],
        ).about("Compare Git-bound improvement hypotheses across supported language producers"))
        .subcommand(benchmark)
        .subcommand(configured_graph)
        .subcommand(kernel)
        .subcommand(agent)
        .subcommand(investigation)
        .subcommand(leaf("mcp", &[], &["repo","roots","db"])
            .about("Serve read-only, repository-scoped MCP tools over stdio"))
        .subcommand(tests)
        .subcommand(
            leaf("browse", &["repo", "manifest"], &["max-bytes"])
                .about("Read-only virtual source browser").hide(true),
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

/// Render names and descriptions from the same clap grammar used for execution.
pub(super) fn catalog() -> String {
    const EVERYDAY: &[&str] = &[
        "analyze",
        "impact",
        "explain",
        "changes",
        "discover",
        "tests",
        "agent",
        "investigation",
        "mcp",
        "doctor",
    ];
    let mut everyday = Vec::new();
    let mut specialist = Vec::new();
    let tree = command();
    for sub in tree.get_subcommands() {
        let name = sub.get_name();
        if name == "commands" {
            continue;
        }
        let description = sub.get_about().map(ToString::to_string).unwrap_or_default();
        let entry = format!("  {name:<14} {description}");
        if EVERYDAY.contains(&name) {
            everyday.push(entry);
        } else {
            specialist.push(entry);
        }
    }
    format!(
        "Everyday\n{}\n\nSpecialist\n{}\n\nRun 'excession <command> --help' for arguments.",
        everyday.join("\n"),
        specialist.join("\n")
    )
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
    fn explain_accepts_one_source_or_bounded_batch_not_legacy_claim_ids() {
        let selected = parse(&strings(&[
            "explain",
            "--report",
            "/tmp/report.json",
            "--source",
            "pkg/affected.py",
            "--source-commit",
            "commit",
        ]))
        .unwrap();
        let ParseOutcome::Command(parsed) = selected else {
            panic!("source witness CLI must parse as a command");
        };
        assert_eq!(parsed.options.get("source"), "pkg/affected.py");
        assert_eq!(parsed.options.get("source-commit"), "commit");
        assert!(
            parse(&strings(&[
                "explain",
                "--report",
                "/tmp/report.json",
                "--all-sources",
                "--json",
            ]))
            .is_ok()
        );
        for args in [
            vec!["explain", "--report", "/tmp/report.json"],
            vec![
                "explain",
                "--report",
                "/tmp/report.json",
                "--id",
                "claim-id",
            ],
            vec![
                "explain",
                "--report",
                "/tmp/report.json",
                "--all-sources",
                "--source",
                "pkg/a.py",
            ],
            vec![
                "explain",
                "--report",
                "/tmp/report.json",
                "--source-commit",
                "commit",
                "--all-sources",
            ],
        ] {
            assert!(parse(&strings(&args)).is_err(), "{args:?}");
        }
    }

    #[test]
    fn everyday_help_is_focused_but_specialist_commands_are_discoverable() {
        let ParseOutcome::Help(help) = parse(&strings(&[])).unwrap() else {
            panic!("root help expected");
        };
        assert!(help.contains("excession impact ."));
        assert!(help.contains("commands"));
        assert!(!help.contains("reconstruct"), "{help}");
        let catalog = catalog();
        for family in [
            "analyze",
            "impact",
            "tests",
            "security",
            "reconstruct",
            "refactor",
        ] {
            assert!(catalog.contains(family), "{family} absent from catalog");
        }
        assert!(parse(&strings(&["refactor", "--help"])).is_ok());
    }

    #[test]
    fn common_comparison_commands_have_optional_base() {
        for command in ["impact", "changes"] {
            let ParseOutcome::Command(parsed) = parse(&strings(&[command, "."])).unwrap() else {
                panic!("command expected");
            };
            assert!(!parsed.options.0.contains_key("base"));
        }
    }

    #[test]
    fn source_witness_impact_is_the_only_command_shape() {
        let ParseOutcome::Command(impact) = parse(&strings(&["impact", ".", "--json"])).unwrap()
        else {
            panic!("impact CLI expected");
        };
        assert_eq!(impact.options.get("json"), "true");
        assert!(parse(&strings(&["impact", ".", "--compact"])).is_err());
        assert!(parse(&strings(&["changes", ".", "--compact"])).is_err());
    }

    #[test]
    fn rust_native_trust_flag_is_explicit_and_json_opt_in() {
        let ParseOutcome::Command(parsed) =
            parse(&strings(&["analyze", "rust", "--trusted-local"])).unwrap()
        else {
            panic!("Rust subcommand expected");
        };
        assert_eq!(parsed.path, ["analyze", "rust"]);
        assert_eq!(parsed.options.get("trusted-local"), "true");
        assert!(!parsed.options.0.contains_key("repo"));
        assert!(!parsed.options.0.contains_key("json"));
        let ParseOutcome::Command(legacy) = parse(&strings(&[
            "analyze",
            "rust",
            "--trusted-local",
            "true",
            "--json",
        ]))
        .unwrap() else {
            panic!("Rust explicit-true form expected");
        };
        assert_eq!(legacy.options.get("trusted-local"), "true");
        assert_eq!(legacy.options.get("json"), "true");
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
    fn agent_init_requires_explicit_write_for_files() {
        let ParseOutcome::Command(planned) = parse(&strings(&["agent", "init"])).unwrap() else {
            panic!("expected agent init");
        };
        assert_eq!(planned.path, ["agent", "init"]);
        assert!(!planned.options.0.contains_key("write"));
        let ParseOutcome::Command(writing) =
            parse(&strings(&["agent", "init", "--client", "codex", "--write"])).unwrap()
        else {
            panic!("expected agent init --write");
        };
        assert_eq!(writing.options.get("write"), "true");
        assert!(parse(&strings(&["mcp", "--repo", "."])).is_ok());
    }

    #[test]
    fn agent_locate_requires_task_and_supports_explicit_revision() {
        assert!(parse(&strings(&["agent", "locate", "--repo", "x"])).is_err());
        assert!(parse(&strings(&["agent", "locate", "--task", "worker lease"])).is_err());
        let ParseOutcome::Command(parsed) = parse(&strings(&[
            "agent",
            "locate",
            "--repo",
            "x",
            "--task",
            "worker lease",
            "--rev",
            "HEAD",
            "--limit",
            "8",
            "--json",
        ]))
        .unwrap() else {
            panic!("expected a command");
        };
        assert_eq!(parsed.path, ["agent", "locate"]);
        assert_eq!(parsed.options.get("task"), "worker lease");
        assert!(
            parse(&strings(&[
                "agent",
                "locate",
                "--repo",
                "x",
                "--task",
                "worker lease",
                "--index-segments",
                "256",
            ]))
            .is_ok()
        );
        assert!(
            parse(&strings(&[
                "agent",
                "locate",
                "--repo",
                "x",
                "--task",
                "lease",
                "--unknown",
            ]))
            .is_err()
        );
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
    fn test_workflow_is_short_and_explicit_about_execution() {
        assert!(parse(&strings(&["tests"])).is_ok());
        assert!(parse(&strings(&["test", "--json"])).is_ok());
        assert!(parse(&strings(&["tests", "--doctor", "--json"])).is_ok());
        assert!(parse(&strings(&["tests", "--run", "--verify", "--json"])).is_ok());
        assert!(
            parse(&strings(&[
                "tests",
                "--run",
                "--sandbox-image",
                "example@sha256:abc",
                "--working-tree"
            ]))
            .is_ok()
        );
        assert!(parse(&strings(&["tests", "--unknown"])).is_err());
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
