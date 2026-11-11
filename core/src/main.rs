use relgit::sqlite::Db;
use relgit::{Result, refuse};
use relgit::{
    contracts, effects, git, historical, identity, impact, index, integration, json, optimizer,
    overlap, projection, scoped, security, security_discovery, security_ingest, security_survey,
    slice_evidence,
    transactions, transform,
};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn usage() -> String {
    "Usage:\n  relgit index DB NAME REPOSITORY [REF] [ROOTS_CSV]\n  relgit tree-check DB REPOSITORY_NAME COMMIT\n  relgit carry EXPERIMENT_ROOT [DATABASE] [REFERENCE_REPORT]
  relgit carry-local EXPERIMENT_ROOT [DATABASE]\n  relgit carry-scoped SPEC_JSON --trusted-local [DATABASE]\n  relgit json-check FILE
  relgit impact DB NAME COMMIT MODULE
  relgit review DB NAME OLD NEW
  relgit manifest DB NAME COMMIT
  relgit dependencies DB NAME COMMIT\n  relgit compose-inferred SPEC_JSON DATABASE\n  relgit compose-inferred-audit SPEC_JSON DATABASE
  relgit security-observe PILOT_DIR DATABASE
  relgit security-observe-audit PILOT_DIR DATABASE
  relgit security-discover SCAN_GIT_DIR DATABASE
  relgit security-discover-audit SCAN_GIT_DIR DATABASE
  relgit security-survey SURVEY_GIT_DIR DATABASE
  relgit security-survey-audit SURVEY_GIT_DIR DATABASE
  relgit security-ingest REPO REPORT DATABASE ENGINE
  relgit security-ingest-audit REPO REPORT DATABASE ENGINE\n  relgit slice-evidence-ingest REPO MANIFEST RECEIPT DB\n  relgit slice-evidence-audit REPO MANIFEST RECEIPT DB\n  relgit view-plan-deps DB REPO_NAME REVISION SEED_MODULE\n  relgit view-plan-replay-deps DB REPO_NAME REVISION SEED_MODULE MANIFEST\n  relgit view-plan-security DB SOURCE_REPO RUN_ID\n  relgit view-plan-replay-security DB SOURCE_REPO RUN_ID MANIFEST\n  relgit view-materialize SOURCE_REPO MANIFEST OUT_DIR\n  relgit view-audit SOURCE_REPO MANIFEST OUT_DIR
  relgit view-compose SOURCE_REPO union|intersection|difference LEFT_MANIFEST RIGHT_MANIFEST
  relgit view-compose-replay SOURCE_REPO union|intersection|difference LEFT_MANIFEST RIGHT_MANIFEST OUTPUT_MANIFEST
  relgit view-select SOURCE_REPO MANIFEST role|source-prefix|virtual-prefix VALUE
  relgit view-mount SOURCE_REPO MANIFEST NAMESPACE
  relgit view-mount-replay SOURCE_REPO MANIFEST NAMESPACE OUTPUT_MANIFEST
  relgit view-select-replay SOURCE_REPO MANIFEST role|source-prefix|virtual-prefix VALUE OUTPUT_MANIFEST
  relgit view-ls SOURCE_REPO MANIFEST [VIRTUAL_DIRECTORY]
  relgit view-stat SOURCE_REPO MANIFEST VIRTUAL_PATH
  relgit view-open SOURCE_REPO MANIFEST VIRTUAL_PATH\nErrors are fatal; unsupported semantics are not silently inferred.".into()
}

fn run() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    let Some(action) = args.get(1).map(String::as_str) else {
        return refuse(usage());
    };
    match action {
        "slice-evidence-ingest" | "slice-evidence-audit" => {
            if args.len() != 6 {
                return refuse("Usage: relgit slice-evidence-ingest|slice-evidence-audit REPO MANIFEST RECEIPT DATABASE");
            }
            let db = Db::open(Path::new(&args[5]))?;
            let result = slice_evidence::evidence(
                Path::new(&args[2]),
                Path::new(&args[3]),
                Path::new(&args[4]),
                &db,
                action == "slice-evidence-audit",
            )?;
            println!("{}", result.canonical());
        }
        "view-compose" | "view-compose-replay" => {
            if args.len() != if action == "view-compose" { 6 } else { 7 } {
                return refuse(
                    "Usage: relgit view-compose[-replay] SOURCE_REPO OP LEFT_MANIFEST RIGHT_MANIFEST [OUTPUT_MANIFEST]",
                );
            }
            let repo = Path::new(&args[2]);
            let left = Path::new(&args[4]);
            let right = Path::new(&args[5]);
            let result = if action == "view-compose" {
                projection::compose(repo, &args[3], left, right)?
            } else {
                projection::compose_replay(repo, &args[3], left, right, Path::new(&args[6]))?
            };
            println!("{}", result.canonical());
        }
        "view-select" | "view-select-replay" => {
            if args.len() != if action == "view-select" { 6 } else { 7 } {
                return refuse(
                    "Usage: relgit view-select[-replay] SOURCE_REPO MANIFEST AXIS VALUE [OUTPUT_MANIFEST]",
                );
            }
            let repo = Path::new(&args[2]);
            let input = Path::new(&args[3]);
            let result = if action == "view-select" {
                projection::select(repo, input, &args[4], &args[5])?
            } else {
                projection::select_replay(repo, input, &args[4], &args[5], Path::new(&args[6]))?
            };
            println!("{}", result.canonical());
        }
        "view-mount" | "view-mount-replay" => {
            if args.len() != if action == "view-mount" { 5 } else { 6 } {
                return refuse(
                    "Usage: relgit view-mount[-replay] SOURCE_REPO MANIFEST NAMESPACE [OUTPUT_MANIFEST]",
                );
            }
            let repo = Path::new(&args[2]);
            let input = Path::new(&args[3]);
            let result = if action == "view-mount" {
                projection::mount(repo, input, &args[4])?
            } else {
                projection::mount_replay(repo, input, &args[4], Path::new(&args[5]))?
            };
            println!("{}", result.canonical());
        }
        "view-ls" => {
            if args.len() != 4 && args.len() != 5 {
                return refuse("Usage: relgit view-ls SOURCE_REPO MANIFEST [VIRTUAL_DIRECTORY]");
            }
            println!(
                "{}",
                projection::list(
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                    args.get(4).map(String::as_str).unwrap_or("")
                )?
                .canonical()
            );
        }
        "view-stat" | "view-open" => {
            if args.len() != 5 {
                return refuse(
                    "Usage: relgit view-stat|view-open SOURCE_REPO MANIFEST VIRTUAL_PATH",
                );
            }
            if action == "view-stat" {
                println!(
                    "{}",
                    projection::describe(Path::new(&args[2]), Path::new(&args[3]), &args[4])?
                        .canonical()
                );
            } else {
                let bytes = projection::open(Path::new(&args[2]), Path::new(&args[3]), &args[4])?;
                io::stdout().write_all(&bytes).map_err(|e| e.to_string())?;
            }
        }
        "view-plan-deps" | "view-plan-replay-deps" => {
            if args.len() != if action == "view-plan-deps" { 6 } else { 7 } {
                return refuse(
                    "Usage: relgit view-plan[-replay]-deps DB REPO_NAME REVISION SEED_MODULE [MANIFEST]",
                );
            }
            let db = Db::open(Path::new(&args[2]))?;
            let expected = projection::dependencies(&db, &args[3], &args[4], &args[5])?;
            let result = if action == "view-plan-replay-deps" {
                projection::replay(&expected, Path::new(&args[6]))?
            } else {
                expected
            };
            println!("{}", result.canonical());
        }
        "view-plan-security" | "view-plan-replay-security" => {
            if args.len() != if action == "view-plan-security" { 5 } else { 6 } {
                return refuse(
                    "Usage: relgit view-plan[-replay]-security DB SOURCE_REPO RUN_ID [MANIFEST]",
                );
            }
            let db = Db::open(Path::new(&args[2]))?;
            let expected = projection::security(&db, Path::new(&args[3]), &args[4])?;
            let result = if action == "view-plan-replay-security" {
                projection::replay(&expected, Path::new(&args[5]))?
            } else {
                expected
            };
            println!("{}", result.canonical());
        }
        "view-materialize" | "view-audit" => {
            if args.len() != 5 {
                return refuse(
                    "Usage: relgit view-materialize|view-audit SOURCE_REPO MANIFEST OUT_DIR",
                );
            }
            let report = if action == "view-materialize" {
                projection::materialize(
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                    Path::new(&args[4]),
                )?
            } else {
                projection::audit(
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                    Path::new(&args[4]),
                )?
            };
            println!("{}", report.canonical());
        }
        "security-ingest" | "security-ingest-audit" => {
            if args.len() != 6 {
                return refuse("Usage: relgit security-ingest[-audit] REPO REPORT DATABASE ENGINE");
            }
            let db = Db::open(Path::new(&args[4]))?;
            println!(
                "{}",
                security_ingest::execute(
                    Path::new(&args[2]),
                    Path::new(&args[3]),
                    &db,
                    &args[5],
                    action == "security-ingest-audit"
                )?
                .canonical()
            );
        }
        "security-survey" | "security-survey-audit" => {
            if args.len() != 4 {
                return refuse("Usage: relgit security-survey[-audit] SURVEY_GIT_DIR DATABASE");
            }
            let db = Db::open(Path::new(&args[3]))?;
            println!(
                "{}",
                security_survey::execute(
                    Path::new(&args[2]),
                    &db,
                    action == "security-survey-audit"
                )?
                .canonical()
            );
        }
        "security-discover" | "security-discover-audit" => {
            if args.len() != 4 {
                return refuse("Usage: relgit security-discover[-audit] SCAN_GIT_DIR DATABASE");
            }
            let db = Db::open(Path::new(&args[3]))?;
            println!(
                "{}",
                security_discovery::execute(
                    Path::new(&args[2]),
                    &db,
                    action == "security-discover-audit"
                )?
                .canonical()
            );
        }
        "security-observe" | "security-observe-audit" => {
            if args.len() != 4 {
                return refuse("Usage: relgit security-observe[-audit] PILOT_DIR DATABASE");
            }
            let db = Db::open(Path::new(&args[3]))?;
            println!(
                "{}",
                security::execute(Path::new(&args[2]), &db, action == "security-observe-audit")?
                    .canonical()
            );
        }
        "effects-observe" | "effects-observe-audit" => {
            if args.len() != 4 {
                return refuse("Usage: relgit effects-observe[-audit] SPEC_JSON DATABASE");
            }
            let spec = effects::Spec::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!(
                "{}",
                effects::execute(&db, &spec, action == "effects-observe-audit")?.canonical()
            );
        }
        "index" => {
            if !(args.len() >= 5 && args.len() <= 7) {
                return refuse(usage());
            }
            let db = Db::open(Path::new(&args[2]))?;
            let rootstr = args.get(6).map(String::as_str).unwrap_or("h11,tests");
            let roots = rootstr
                .split(',')
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>();
            if roots.is_empty() {
                return refuse("at least one module root required");
            }
            let id = index::index_revision_with_roots(
                &db,
                Path::new(&args[4]),
                &args[3],
                args.get(5).map(String::as_str).unwrap_or("HEAD"),
                "local-checkout",
                &roots,
            )?;
            let refspec = git::rev(
                Path::new(&args[4]),
                args.get(5).map(String::as_str).unwrap_or("HEAD"),
            )?;
            let root = index::reconstruct_tree(&db, id, &refspec)?;
            println!("indexed repo={} commit={refspec} root_tree={root}", args[3]);
        }
        "tree-check" => {
            if args.len() != 5 {
                return refuse(usage());
            }
            let db = Db::open(Path::new(&args[2]))?;
            let id = db
                .scalar(
                    "SELECT id FROM repository WHERE name=?",
                    &[relgit::sqlite::Param::Text(&args[3])],
                )?
                .parse::<i64>()
                .map_err(|e| e.to_string())?;
            let tree = index::reconstruct_tree(&db, id, &args[4])?;
            db.foreign_keys_valid()?;
            println!("verified root_tree={tree}");
        }
        "carry" => {
            if !(args.len() >= 3 && args.len() <= 5) {
                return refuse(usage());
            }
            let root = PathBuf::from(&args[2])
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let observations = integration::Observations::from_files(&root)?;
            let db_path = args.get(3).map(PathBuf::from);
            let result = integration::bounded_carry(&root, &observations, db_path.as_deref())?;
            if let Some(path) = args.get(4) {
                let reference = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
                integration::compare_reference(&result, &reference)?;
            }
            println!("{}", result.canonical());
        }
        "carry-local" => {
            if !(args.len() == 3 || args.len() == 4) {
                return refuse("Usage: relgit carry-local EXPERIMENT_ROOT [DATABASE]");
            }
            let root = PathBuf::from(&args[2])
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let db_path = args.get(3).map(PathBuf::from);
            let result = integration::carry_local(&root, db_path.as_deref())?;
            println!("{}", result.canonical());
        }
        "carry-scoped" => {
            if !(args.len() == 4 || args.len() == 5) || args[3] != "--trusted-local" {
                return refuse("Usage: relgit carry-scoped SPEC_JSON --trusted-local [DATABASE]");
            }
            let db = args.get(4).map(PathBuf::from);
            let result = scoped::carry_scoped(Path::new(&args[2]), db.as_deref(), true)?;
            println!("{}", result.canonical());
        }
        "contract-impact" => {
            if args.len() != 4 {
                return refuse("Usage: relgit contract-impact SPEC_JSON DATABASE");
            }
            let spec = impact::ImpactPlan::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!("{}", impact::assess(&db, &spec)?.canonical());
        }
        "identity" => {
            if args.len() != 4 {
                return refuse("Usage: relgit identity SPEC_JSON DATABASE");
            }
            let spec = identity::IdentityPlan::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!("{}", identity::materialize(&db, &spec)?.canonical());
        }
        "compose-overlap"
        | "compose-overlap-audit"
        | "compose-operation"
        | "compose-operation-audit"
        | "compose-inferred"
        | "compose-inferred-audit" => {
            if args.len() != 4 {
                return refuse(
                    "Usage: relgit compose-overlap[-audit] or compose-operation[-audit] SPEC_JSON DATABASE",
                );
            }
            let spec = overlap::Spec::read(Path::new(&args[2]))?;
            if action.starts_with("compose-operation") != spec.is_operation()
                || action.starts_with("compose-inferred") != spec.is_inferred()
            {
                return refuse("command must match inferred, typed-operation, or slot mode");
            }
            let db = Db::open(Path::new(&args[3]))?;
            let result = if matches!(
                action,
                "compose-overlap" | "compose-operation" | "compose-inferred"
            ) {
                overlap::compose(&db, &spec)?
            } else {
                overlap::audit(&db, &spec)?
            };
            println!("{}", result.canonical());
        }
        "compose-audit" => {
            if args.len() != 4 {
                return refuse("Usage: relgit compose-audit SPEC_JSON DATABASE");
            }
            let spec = transactions::Spec::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!("{}", transactions::audit(&db, &spec)?.canonical());
        }
        "compose" => {
            if args.len() != 4 {
                return refuse("Usage: relgit compose SPEC_JSON DATABASE");
            }
            let spec = transactions::Spec::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!("{}", transactions::compose(&db, &spec)?.canonical());
        }
        "contracts-import" => {
            if args.len() != 4 {
                return refuse("Usage: relgit contracts-import SPEC_JSON DATABASE");
            }
            let spec = contracts::Spec::read(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            println!("{}", contracts::import(&db, &spec)?.canonical());
        }
        "replay" => {
            if args.len() != 4 {
                return refuse("Usage: relgit replay SPEC_JSON DATABASE");
            }
            let plan = historical::from_spec(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            let result = historical::replay(&db, &plan)?;
            println!("{}", result.canonical());
        }
        "score" => {
            if args.len() != 3 {
                return refuse("Usage: relgit score COST_INPUT_JSON");
            }
            println!("{}", optimizer::from_file(Path::new(&args[2]))?.canonical());
        }
        "project" => {
            if args.len() != 5 {
                return refuse("Usage: relgit project SPEC_JSON DATABASE OUTPUT_DIRECTORY");
            }
            let plan = transform::Plan::parse(Path::new(&args[2]))?;
            let db = Db::open(Path::new(&args[3]))?;
            let report = transform::project(&db, &plan, Path::new(&args[4]))?;
            println!("{}", report.canonical());
        }
        "json-check" => {
            if args.len() != 3 {
                return refuse(usage());
            }
            let result = json::parse(&fs::read(&args[2]).map_err(|e| e.to_string())?)?;
            println!("canonical_sha256={}", result.sha256());
        }
        "impact" | "review" | "manifest" | "dependencies" => {
            let expected = if action == "impact" || action == "review" {
                6
            } else {
                5
            };
            if args.len() != expected {
                return refuse(usage());
            }
            let db = Db::open(Path::new(&args[2]))?;
            let output = match action {
                "impact" => relgit::views::impact(&db, &args[3], &args[4], &args[5])?,
                "review" => relgit::views::review(&db, &args[3], &args[4], &args[5])?,
                "manifest" => relgit::views::manifest(&db, &args[3], &args[4])?,
                "dependencies" => relgit::views::dependencies(&db, &args[3], &args[4])?,
                _ => unreachable!(),
            };
            println!("{}", output.canonical());
        }
        _ => return refuse(usage()),
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("REFUSED: {e}");
        std::process::exit(1);
    }
}
