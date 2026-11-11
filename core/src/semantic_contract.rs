//! Normalize replayed syntax-resolution candidates into a language-neutral contract.
//! The contract adds no new dependency knowledge and never proves graph completeness.
use crate::json::{self, Value, obj};
use crate::{Result, refuse, syntax_resolution};
use std::fs;
use std::path::Path;

const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

fn anchor(item: &Value) -> Result<Value> {
    Ok(obj(&[
        ("path", item.get("source_path")?.clone()),
        ("blob", item.get("source_blob")?.clone()),
        ("line", item.get("line")?.clone()),
        ("language", item.get("language")?.clone()),
    ]))
}

fn observation(item: &Value, candidate: bool) -> Result<Value> {
    let assessment = obj(&[
        ("status", Value::string(if candidate { "CANDIDATE" } else { "UNRESOLVED" })),
        ("method", Value::string("SYNTAX_RESOLUTION")),
        (
            "rule",
            item.get(if candidate { "resolution" } else { "reason" })?.clone(),
        ),
    ]);
    let mut fields = obj(&[
        ("kind", Value::string("IMPORTS")),
        ("subject", anchor(item)?),
        ("specifier", item.get("specifier")?.clone()),
        ("assessment", assessment),
        ("condition_evaluation", Value::string("NOT_EVALUATED")),
    ]);
    if candidate {
        fields.entries_mut()?.insert(
            "target".into(),
            obj(&[
                ("path", item.get("target_path")?.clone()),
                ("blob", item.get("target_blob")?.clone()),
            ]),
        );
    }
    let id = fields.sha256();
    fields
        .entries_mut()?
        .insert("observation_id".into(), Value::string(id));
    Ok(fields)
}

/// Independent Git re-parsing and resolution replay happens before conversion.
/// The resulting report_id authenticates neither a foreign signer nor a runtime.
pub fn export(repo: &Path, syntax_file: &Path, resolution_file: &Path) -> Result<Value> {
    syntax_resolution::audit(repo, syntax_file, resolution_file)?;
    let raw = fs::read(resolution_file).map_err(|e| format!("resolution read: {e}"))?;
    if raw.len() > MAX_REPORT_BYTES {
        return refuse("resolution report exceeds 32 MiB");
    }
    let resolution = json::parse(&raw)?;
    let mut records = Vec::new();
    for edge in resolution.get("candidate_edges")?.array_ref()? {
        records.push(observation(edge, true)?);
    }
    for missing in resolution.get("unresolved_imports")?.array_ref()? {
        records.push(observation(missing, false)?);
    }
    records.sort_by_key(Value::canonical);

    let mut gaps = resolution.get("skipped_sources")?.array_ref()?.to_vec();
    let syntax_raw = fs::read(syntax_file).map_err(|e| format!("syntax read: {e}"))?;
    if syntax_raw.len() > MAX_REPORT_BYTES {
        return refuse("syntax report exceeds 32 MiB");
    }
    let syntax = json::parse(&syntax_raw)?;
    for source in syntax.get("sources")?.array_ref()? {
        if source.get("status")?.str()? == "PARSED_WITH_ERRORS" {
            gaps.push(obj(&[
                ("source_path", source.get("source_path")?.clone()),
                ("status", Value::string("PARSED_WITH_ERRORS")),
            ]));
        }
    }
    gaps.sort_by_key(Value::canonical);
    gaps.dedup();

    let payload = obj(&[
        ("schema", Value::string("excession.semantic-observations.v1")),
        ("source_commit", resolution.get("source_commit")?.clone()),
        ("source_tree", resolution.get("source_tree")?.clone()),
        ("syntax_report_id", resolution.get("syntax_report_id")?.clone()),
        ("resolution_report_id", resolution.get("resolution_id")?.clone()),
        ("completeness", Value::string("PARTIAL_CANDIDATES_ONLY")),
        ("record_count", Value::number(records.len())),
        ("gap_count", Value::number(gaps.len())),
        ("records", Value::array(records)),
        ("gaps", Value::array(gaps)),
    ]);
    let id = payload.sha256();
    let mut result = payload;
    result
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge() -> Value {
        obj(&[
            ("source_path", Value::string("src/main.rs")),
            ("source_blob", Value::string("a".repeat(40))),
            ("target_path", Value::string("src/util.rs")),
            ("target_blob", Value::string("b".repeat(40))),
            ("language", Value::string("rust")),
            ("specifier", Value::string("crate::util")),
            ("line", Value::number(2)),
            ("resolution", Value::string("RUST_CRATE_PATH_CANDIDATE")),
        ])
    }

    #[test]
    fn candidate_has_target_but_no_verified_claim() {
        let record = observation(&edge(), true).unwrap();
        assert_eq!(record.get("kind").unwrap().str().unwrap(), "IMPORTS");
        assert_eq!(
            record.get("assessment").unwrap().get("status").unwrap().str().unwrap(),
            "CANDIDATE"
        );
        assert_eq!(record.get("target").unwrap().get("path").unwrap().str().unwrap(), "src/util.rs");
        assert_eq!(record.get("condition_evaluation").unwrap().str().unwrap(), "NOT_EVALUATED");
        assert_eq!(record.get("observation_id").unwrap().str().unwrap().len(), 64);
    }

    #[test]
    fn unresolved_does_not_invent_target() {
        let mut item = edge();
        item.entries_mut().unwrap().remove("resolution");
        item.entries_mut().unwrap().insert("reason".into(), Value::string("AMBIGUOUS_TRACKED_TARGETS"));
        let record = observation(&item, false).unwrap();
        assert!(record.get("target").is_err());
        assert_eq!(
            record.get("assessment").unwrap().get("status").unwrap().str().unwrap(),
            "UNRESOLVED"
        );
    }

    #[test]
    fn identities_are_deterministic_and_source_sensitive() {
        let a = observation(&edge(), true).unwrap();
        let b = observation(&edge(), true).unwrap();
        assert_eq!(a.canonical(), b.canonical());
        let mut changed = edge();
        changed.entries_mut().unwrap().insert("target_blob".into(), Value::string("c".repeat(40)));
        assert_ne!(a.get("observation_id").unwrap(), observation(&changed, true).unwrap().get("observation_id").unwrap());
    }
}
