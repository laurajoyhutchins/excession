//! Historical declared-input and receipt transport through the shared Git/SQLite core.
use super::{Options, Response, json};
use excession::sqlite::Db;
use excession::{Result, evidence_transport, input_contract, native_discovery, refuse};

pub(super) fn run(verb: &str, opt: &Options) -> Result<Response> {
    Ok(json(match verb {
        "discover-native" => {
            let trusted = match opt.get("trusted-local") {
                "true" => true,
                "false" => false,
                _ => return refuse("--trusted-local requires true or false"),
            };
            native_discovery::discover(
                opt.path("repo"),
                opt.get("rev"),
                opt.get("entry"),
                opt.get("language"),
                trusted,
            )?
        }
        "contract-plan" => input_contract::plan_file(
            opt.path("repo"),
            opt.get("rev"),
            opt.path("declaration"),
        )?,
        "contract-compare" | "contract-audit" => {
            let db = Db::open(opt.path("db"))?;
            input_contract::transport(
                opt.path("repo"),
                opt.path("source-contract"),
                opt.path("target-contract"),
                &db,
                verb == "contract-audit",
            )?
        }
        "trace" => {
            let db = Db::open(opt.path("db"))?;
            evidence_transport::trace(
                opt.path("repo"),
                opt.get("receipt-id"),
                &db,
            )?
        }
        "transport" | "transport-audit" => {
            let db = Db::open(opt.path("db"))?;
            evidence_transport::transport(
                opt.path("repo"),
                opt.path("source-manifest"),
                opt.path("target-manifest"),
                opt.path("receipt"),
                &db,
                verb == "transport-audit",
            )?
        }
        _ => return refuse(format!("unknown evidence command {verb}")),
    }))
}
