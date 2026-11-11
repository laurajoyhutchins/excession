//! Native executable-slice commands. Clap owns parsing; Rust owns authority.
use super::{Options, Response, json};
use excession::slice_evidence;
use excession::slice_runtime;
use excession::sqlite::Db;
use excession::{Result, refuse};

fn boolean(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => refuse("boolean options require true or false"),
    }
}

pub(super) fn run(verb: &str, opt: &Options) -> Result<Response> {
    Ok(json(match verb {
        "ingest" | "audit" => {
            let db = Db::open(opt.path("db"))?;
            slice_evidence::evidence(
                opt.path("repo"),
                opt.path("manifest"),
                opt.path("receipt"),
                &db,
                verb == "audit",
            )?
        }
        "plan" => slice_runtime::plan_cli(
            opt.path("repo"),
            opt.get("rev"),
            opt.get("entry"),
            opt.get_or("assets", "[]"),
            boolean(opt.get_or("history", "false"))?,
        )?,
        "validate" => slice_runtime::validate_cli(opt.path("repo"), opt.path("manifest"))?,
        "materialize" => slice_runtime::materialize_cli(
            opt.path("repo"),
            opt.path("manifest"),
            opt.path("directory"),
        )?,
        "source-audit" => slice_runtime::audit_cli(
            opt.path("repo"),
            opt.path("manifest"),
            opt.path("directory"),
        )?,
        "run" => {
            let timeout = opt
                .get_or("timeout", "10")
                .parse::<u64>()
                .map_err(|_| "timeout must be a number from 1 to 60")?;
            slice_runtime::run_cli(
                opt.path("repo"),
                opt.path("manifest"),
                opt.path("directory"),
                timeout,
                boolean(opt.get_or("isolated-linux", "false"))?,
                boolean(opt.get("trusted-local"))?,
            )?
        }
        "carry" => slice_runtime::carry_cli(
            opt.path("repo"),
            opt.path("manifest"),
            opt.path("receipt"),
            opt.get("rev"),
            boolean(opt.get_or("replay", "false"))?,
            boolean(opt.get_or("isolated-linux", "false"))?,
            boolean(opt.get_or("trusted-local", "false"))?,
        )?,
        _ => return refuse(format!("unknown slice command {verb}")),
    }))
}
