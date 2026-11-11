//! Thin interactive command router. Providers and evidence semantics live in Rust core.
//! The CLI parsing surface will share clap with the in-progress CLI migration.
use super::{Options, Response, json};
use excession::sqlite::Db;
use excession::{Result, graph};
use std::path::Path;

fn database(opt: &Options) -> Result<(Db, String)> {
    let path = graph::database_path(
        opt.0.get("db").map(|p| Path::new(p)),
        opt.0.get("build").map(|p| Path::new(p)),
    )?;
    let db = Db::open(&path)?;
    Ok((db, path.to_string_lossy().into_owned()))
}
fn depth(opt: &Options, default: usize) -> Result<usize> {
    opt.get_or("depth", if default == 8 { "8" } else { "2" })
        .parse::<usize>().map_err(|_| "invalid graph query depth".into())
}
fn optional<'a>(opt: &'a Options, key: &str) -> Option<&'a str> {
    opt.0.get(key).map(String::as_str)
}
pub(super) fn run(verb: &str, argv: &[String]) -> Result<Response> {
    Ok(match verb {
        "import" => {
            let opt = Options::parse(argv, &["build", "target"],
                &["repo", "rev", "db", "provider", "observations", "trusted-local", "ar"])?;
            let (db, path) = database(&opt)?;
            let input = graph::Import {
                repo: Path::new(opt.get_or("repo", ".")),
                revision: opt.get_or("rev", "HEAD"),
                build: opt.path("build"),
                target: opt.get("target"),
                provider: opt.get_or("provider", "auto"),
                observations: optional(&opt, "observations").map(Path::new),
                trusted_local: opt.get_or("trusted-local", "false"),
                archiver: opt.get_or("ar", "ar"),
            };
            let mut result = graph::import(&db, &input)?;
            result.entries_mut()?.insert("database".to_owned(),
                excession::json::Value::string(path));
            json(result)
        }
        "ls" => {
            let opt = Options::parse(argv, &[], &["db", "build"])?;
            let (db, _) = database(&opt)?;
            json(graph::list(&db)?)
        }
        "show" => {
            let opt = Options::parse(argv, &[],
                &["db", "build", "id", "target"])?;
            let (db, _) = database(&opt)?;
            json(graph::show(&db, optional(&opt, "id"), optional(&opt, "target"))?)
        }
        "deps" => {
            let opt = Options::parse(argv, &["node"],
                &["db", "build", "id", "target", "direction", "depth"])?;
            let (db, _) = database(&opt)?;
            json(graph::neighbors(
                &db, optional(&opt, "id"), optional(&opt, "target"), opt.get("node"),
                opt.get_or("direction", "out"), depth(&opt, 2)?,
            )?)
        }
        "path" => {
            let opt = Options::parse(argv, &["from", "to"],
                &["db", "build", "id", "target", "depth"])?;
            let (db, _) = database(&opt)?;
            json(graph::path(
                &db, optional(&opt, "id"), optional(&opt, "target"),
                opt.get("from"), opt.get("to"), depth(&opt, 8)?,
            )?)
        }
        _ => return excession::refuse("unknown graph verb"),
    })
}
