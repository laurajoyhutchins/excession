//! Normalize replayed syntax-resolution candidates into a language-neutral contract.
//! The contract adds no new dependency knowledge and never proves graph completeness.
use crate::json::{self, Value, obj};
use crate::syntax_resolution::{ImportEvent, ImportOutcome};
use crate::{Result, git, refuse, syntax, syntax_resolution};
use std::fs;
use std::path::Path;

const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

fn make_observation(
    source_path: &str,
    source_blob: &str,
    line: i64,
    language: &str,
    specifier: &str,
    target: Option<(&str, &str)>,
    rule: &str,
) -> Value {
    let assessment = obj(&[
        (
            "status",
            Value::string(if target.is_some() {
                "CANDIDATE"
            } else {
                "UNRESOLVED"
            }),
        ),
        ("method", Value::string("SYNTAX_RESOLUTION")),
        ("rule", Value::string(rule)),
    ]);
    let mut fields = obj(&[
        ("kind", Value::string("IMPORTS")),
        (
            "subject",
            obj(&[
                ("path", Value::string(source_path)),
                ("blob", Value::string(source_blob)),
                ("line", Value::number(line)),
                ("language", Value::string(language)),
            ]),
        ),
        ("specifier", Value::string(specifier)),
        ("assessment", assessment),
        ("condition_evaluation", Value::string("NOT_EVALUATED")),
    ]);
    if let Some((path, blob)) = target {
        fields
            .entries_mut()
            .expect("object constructed above")
            .insert(
                "target".into(),
                obj(&[("path", Value::string(path)), ("blob", Value::string(blob))]),
            );
    }
    let id = fields.sha256();
    fields
        .entries_mut()
        .expect("object constructed above")
        .insert("observation_id".into(), Value::string(id));
    fields
}

/// Direct shared serialization from the typed decision made by a
/// language-specific resolver, without decoding its legacy report.
fn observation_event(event: &ImportEvent) -> Value {
    let (target, rule) = match &event.outcome {
        ImportOutcome::Candidate {
            target_path,
            target_blob,
            rule,
        } => (Some((target_path.as_str(), target_blob.as_str())), *rule),
        ImportOutcome::Unresolved { reason } => (None, *reason),
    };
    make_observation(
        &event.source_path,
        &event.source_blob,
        event.line,
        &event.language,
        &event.specifier,
        target,
        rule,
    )
}

/// Compatibility path when a saved historical candidate report has been
/// independently regenerated and compared to pinned Git source objects.
fn observation(item: &Value, candidate: bool) -> Result<Value> {
    let target = if candidate {
        Some((
            item.get("target_path")?.str()?,
            item.get("target_blob")?.str()?,
        ))
    } else {
        None
    };
    let rule = item
        .get(if candidate { "resolution" } else { "reason" })?
        .str()?;
    Ok(make_observation(
        item.get("source_path")?.str()?,
        item.get("source_blob")?.str()?,
        item.get("line")?.integer()?,
        item.get("language")?.str()?,
        item.get("specifier")?.str()?,
        target,
        rule,
    ))
}

fn read_report(path: &Path, label: &str) -> Result<Value> {
    let raw = fs::read(path).map_err(|e| format!("{label} read: {e}"))?;
    if raw.len() > MAX_REPORT_BYTES {
        return refuse(format!("{label} report exceeds 32 MiB"));
    }
    json::parse(&raw)
}

/// Run trusted, locally installed language parsers against pinned Git blobs,
/// then emit the shared semantic contract without any temporary report files.
/// This is a fresh observation, not an independently replayed audit.
pub fn scan(repo: &Path, reference: &str, roots: &[&str]) -> Result<Value> {
    let syntax = syntax::scan(repo, reference, roots)?;
    let mut records = Vec::new();
    let resolution = syntax_resolution::candidates_from_scanned(repo, &syntax, |event| {
        records.push(observation_event(event));
        Ok(())
    })?;
    normalize_records(&resolution, &syntax, records)
}

/// Independently rerun a direct scan against its pinned source revision.
/// Roots are explicit to ensure the reconstructed parser scope is equivalent.
pub fn audit_scan(repo: &Path, roots: &[&str], report_file: &Path) -> Result<Value> {
    let stored = read_report(report_file, "semantic")?;
    let reference = git::oid(stored.get("source_commit")?.str()?)?;
    let generated = scan(repo, &reference, roots)?;
    if stored.canonical() != generated.canonical() {
        return refuse("semantic observation report differs from independently replayed Git scan");
    }
    Ok(obj(&[
        ("report_id", generated.get("report_id")?.clone()),
        ("record_count", generated.get("record_count")?.clone()),
        ("gap_count", generated.get("gap_count")?.clone()),
        ("status", Value::string("REGENERATED_MATCH")),
    ]))
}

/// Source-bound relationship adapters emit semantic observations at the moment
/// each language-specific import is resolved. The legacy resolution manifest
/// is still computed in memory to preserve its stable resolution ID.
pub fn resolve(repo: &Path, syntax_file: &Path) -> Result<Value> {
    let mut records = Vec::new();
    let resolution = syntax_resolution::candidates_with_sink(repo, syntax_file, |event| {
        records.push(observation_event(event));
        Ok(())
    })?;
    let syntax = read_report(syntax_file, "syntax")?;
    normalize_records(&resolution, &syntax, records)
}

/// Recheck a previously saved resolution report before producing identical
/// semantic observations. A matching unkeyed digest is not attestation.
pub fn export(repo: &Path, syntax_file: &Path, resolution_file: &Path) -> Result<Value> {
    syntax_resolution::audit(repo, syntax_file, resolution_file)?;
    let resolution = read_report(resolution_file, "resolution")?;
    let syntax = read_report(syntax_file, "syntax")?;
    normalize(&resolution, &syntax)
}

// Compatibility path for independently audited, previously saved candidate reports.
fn normalize(resolution: &Value, syntax: &Value) -> Result<Value> {
    let mut records = Vec::new();
    for edge in resolution.get("candidate_edges")?.array_ref()? {
        records.push(observation(edge, true)?);
    }
    for missing in resolution.get("unresolved_imports")?.array_ref()? {
        records.push(observation(missing, false)?);
    }
    normalize_records(resolution, syntax, records)
}

fn normalize_records(resolution: &Value, syntax: &Value, mut records: Vec<Value>) -> Result<Value> {
    records.sort_by_key(Value::canonical);
    // Repeated syntactic imports of the same source/target relationship are
    // identical observations, not independent evidence or a higher count.
    records.dedup();

    let mut gaps = resolution.get("skipped_sources")?.array_ref()?.to_vec();
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
        (
            "schema",
            Value::string("excession.semantic-observations.v1"),
        ),
        ("source_commit", resolution.get("source_commit")?.clone()),
        ("source_tree", resolution.get("source_tree")?.clone()),
        (
            "syntax_report_id",
            resolution.get("syntax_report_id")?.clone(),
        ),
        (
            "resolution_report_id",
            resolution.get("resolution_id")?.clone(),
        ),
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
    fn normalization_keeps_partial_parse_gap() {
        let resolution = obj(&[
            ("candidate_edges", Value::array(vec![edge()])),
            ("unresolved_imports", Value::array(vec![])),
            ("skipped_sources", Value::array(vec![])),
            ("source_commit", Value::string("c".repeat(40))),
            ("source_tree", Value::string("d".repeat(40))),
            ("syntax_report_id", Value::string("e".repeat(64))),
            ("resolution_id", Value::string("f".repeat(64))),
        ]);
        let syntax = obj(&[(
            "sources",
            Value::array(vec![obj(&[
                ("source_path", Value::string("src/main.rs")),
                ("status", Value::string("PARSED_WITH_ERRORS")),
            ])]),
        )]);
        let report = normalize(&resolution, &syntax).unwrap();
        assert_eq!(report.get("record_count").unwrap().integer().unwrap(), 1);
        assert_eq!(report.get("gap_count").unwrap().integer().unwrap(), 1);
        assert_eq!(
            report.get("completeness").unwrap().str().unwrap(),
            "PARTIAL_CANDIDATES_ONLY"
        );
    }

    #[test]
    fn typed_adapters_match_historical_candidate_encoding() {
        let direct = ImportEvent {
            source_path: "src/main.rs".into(),
            source_blob: "a".repeat(40),
            language: "rust".into(),
            specifier: "crate::util".into(),
            line: 2,
            outcome: ImportOutcome::Candidate {
                target_path: "src/util.rs".into(),
                target_blob: "b".repeat(40),
                rule: "RUST_CRATE_PATH_CANDIDATE",
            },
        };
        assert_eq!(
            observation_event(&direct),
            observation(&edge(), true).unwrap()
        );

        let unresolved = ImportEvent {
            outcome: ImportOutcome::Unresolved {
                reason: "AMBIGUOUS_TRACKED_TARGETS",
            },
            ..direct
        };
        let mut legacy = edge();
        let fields = legacy.entries_mut().unwrap();
        fields.remove("target_path");
        fields.remove("target_blob");
        fields.remove("resolution");
        fields.insert("reason".into(), Value::string("AMBIGUOUS_TRACKED_TARGETS"));
        assert_eq!(
            observation_event(&unresolved),
            observation(&legacy, false).unwrap()
        );
    }

    #[test]
    fn candidate_has_target_but_no_verified_claim() {
        let record = observation(&edge(), true).unwrap();
        assert_eq!(record.get("kind").unwrap().str().unwrap(), "IMPORTS");
        assert_eq!(
            record
                .get("assessment")
                .unwrap()
                .get("status")
                .unwrap()
                .str()
                .unwrap(),
            "CANDIDATE"
        );
        assert_eq!(
            record
                .get("target")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/util.rs"
        );
        assert_eq!(
            record.get("condition_evaluation").unwrap().str().unwrap(),
            "NOT_EVALUATED"
        );
        assert_eq!(
            record.get("observation_id").unwrap().str().unwrap().len(),
            64
        );
    }

    #[test]
    fn unresolved_does_not_invent_target() {
        let mut item = edge();
        item.entries_mut().unwrap().remove("resolution");
        item.entries_mut()
            .unwrap()
            .insert("reason".into(), Value::string("AMBIGUOUS_TRACKED_TARGETS"));
        let record = observation(&item, false).unwrap();
        assert!(record.get("target").is_err());
        assert_eq!(
            record
                .get("assessment")
                .unwrap()
                .get("status")
                .unwrap()
                .str()
                .unwrap(),
            "UNRESOLVED"
        );
    }

    #[test]
    fn identities_are_deterministic_and_source_sensitive() {
        let a = observation(&edge(), true).unwrap();
        let b = observation(&edge(), true).unwrap();
        assert_eq!(a.canonical(), b.canonical());
        let mut changed = edge();
        changed
            .entries_mut()
            .unwrap()
            .insert("target_blob".into(), Value::string("c".repeat(40)));
        assert_ne!(
            a.get("observation_id").unwrap(),
            observation(&changed, true)
                .unwrap()
                .get("observation_id")
                .unwrap()
        );
    }
}
