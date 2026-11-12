//! Bounded source-bound impact investigation over existing static candidates.
//! A candidate import path never establishes change breakage or safe test elision.
use crate::analyze;
use crate::change_inventory;
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

const MAX_DEPTH: usize = 8;
const MAX_EFFECTS: usize = 512;
const MAX_GAPS: usize = 9999;

/// Return each producer's native relationship and its evidence unchanged.
/// Current aggregate analysis only emits static import *candidates*.
fn incoming(analysis: &Value) -> Result<BTreeMap<String, Vec<Value>>> {
    let mut by_target = BTreeMap::<String, Vec<Value>>::new();
    for edge in analysis.get("relationships")?.array_ref()? {
        let kind = edge.get("kind")?.str()?;
        if kind != "STATIC_IMPORT" {
            return refuse(format!(
                "impact cannot silently interpret unsupported relationship kind {kind}"
            ));
        }
        let target = edge.get("target")?.get("path")?.str()?;
        let evidences = edge.get("evidence")?.array_ref()?;
        if evidences.is_empty() {
            return refuse("impact relationship lacks source evidence");
        }
        // Imported module edges remain candidates until an independent
        // semantic oracle establishes anything stronger.
        if evidences
            .iter()
            .any(|e| e.get("assessment").and_then(Value::str).ok() != Some("CANDIDATE"))
        {
            return refuse("static dependency candidate has an invalid evidence assessment");
        }
        by_target.entry(target.to_owned()).or_default().push(obj(&[
            ("source", edge.get("source")?.clone()),
            ("target", edge.get("target")?.clone()),
            ("relation", Value::string(kind)),
            ("evidence", edge.get("evidence")?.clone()),
        ]));
    }
    Ok(by_target)
}

fn changed_paths(inventory: &Value, side: &str) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for change in inventory.get("changes")?.array_ref()? {
        let source = change.get(side)?;
        if source == &Value::Null {
            continue;
        }
        paths.insert(source.get("path")?.str()?.to_owned());
    }
    Ok(paths)
}

/// Reversed BFS from a changed source: every returned ordered path flows
/// from an importer toward the changed module, with each hop's original
/// source and producer observation intact.
fn walk(
    incoming: &BTreeMap<String, Vec<Value>>,
    changed: &BTreeSet<String>,
    effects: &mut Vec<Value>,
) -> Result<bool> {
    for seed in changed {
        // An importer may depend on the changed module through more than one
        // route. A global visited set loses distinct evidence chains in a
        // diamond graph; only forbid cycles *within the current path*.
        let visited = BTreeSet::from([seed.to_owned()]);
        let mut queue = VecDeque::from([(seed.to_owned(), Vec::<Value>::new(), visited)]);
        while let Some((target, suffix, visited)) = queue.pop_front() {
            if let Some(edges) = incoming.get(&target) {
                for edge in edges {
                    let path = edge.get("source")?.get("path")?.str()?.to_owned();
                    if visited.contains(&path) {
                        continue;
                    }
                    if suffix.len() >= MAX_DEPTH {
                        return Ok(true);
                    }
                    let mut hops = Vec::with_capacity(suffix.len() + 1);
                    hops.push(edge.clone());
                    hops.extend(suffix.iter().cloned());
                    let entity = edge.get("source")?.clone();
                    let mut effect = obj(&[
                        ("entity", entity),
                        ("assessment", Value::string("CANDIDATE_PATH")),
                        ("hops", Value::array(hops.clone())),
                    ]);
                    let claim_id = effect.sha256();
                    effect
                        .entries_mut()?
                        .insert("claim_id".to_owned(), Value::string(claim_id));
                    effects.push(effect);
                    if effects.len() >= MAX_EFFECTS {
                        return Ok(true);
                    }
                    let mut next_visited = visited.clone();
                    next_visited.insert(path.clone());
                    queue.push_back((path, hops, next_visited));
                }
            }
        }
    }
    Ok(false)
}

fn indexed_language(path: &str) -> bool {
    [".py", ".js", ".cjs", ".mjs", ".ts"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn observe(
    path: &Path,
    rev: &str,
    roots: Option<&str>,
    database: Option<&str>,
) -> Result<Option<Value>> {
    Ok(analyze::run_if_supported(path, rev, roots, database)?.map(|(report, _)| report))
}

/// Investigate both revision graphs. A removed source is considered under the
/// base revision rather than being invented in the head revision.
pub fn run(
    path: &Path,
    base: &str,
    head: &str,
    roots: Option<&str>,
    database: Option<&str>,
) -> Result<Value> {
    let changes = change_inventory::inventory(path, base, head)?;
    let base_commit = changes.get("base_commit")?.str()?;
    let head_commit = changes.get("head_commit")?.str()?;
    let mut effects = Vec::new();
    let mut gaps = Vec::new();
    let mut source_count = 0i64;
    let mut truncated = false;
    for (revision, side) in [(base_commit, "before"), (head_commit, "after")] {
        let changed = changed_paths(&changes, side)?;
        for source in &changed {
            if !indexed_language(source) {
                gaps.push(obj(&[
                    ("reason", Value::string("UNSUPPORTED_LANGUAGE")),
                    ("path", Value::string(source)),
                ]));
            }
        }
        if changed.is_empty() {
            continue;
        }
        let Some(report) = observe(path, revision, roots, database)? else {
            gaps.push(obj(&[
                ("reason", Value::string("EXCLUDED_SCOPE")),
                (
                    "detail",
                    Value::string("no supported static module roots in selected revision"),
                ),
            ]));
            continue;
        };
        source_count = source_count
            .checked_add(report.get("coverage")?.get("analyzed_sources")?.integer()?)
            .ok_or("analyzed source count overflow")?;
        gaps.extend(
            report
                .get("coverage")?
                .get("gaps")?
                .array_ref()?
                .iter()
                .cloned(),
        );
        truncated |= walk(&incoming(&report)?, &changed, &mut effects)?;
        if truncated {
            break;
        }
    }
    if gaps.len() > MAX_GAPS {
        gaps.truncate(MAX_GAPS);
        truncated = true;
    }
    if truncated {
        gaps.push(obj(&[("reason", Value::string("TRUNCATED"))]));
    }
    let mut result = obj(&[
        ("schema", Value::string("excession.impact.v1")),
        ("base_commit", Value::string(base_commit)),
        ("head_commit", Value::string(head_commit)),
        ("change_inventory_id", changes.get("report_id")?.clone()),
        ("effects", Value::array(effects)),
        ("unknown", Value::array(gaps.clone())),
        (
            "coverage",
            obj(&[
                ("status", Value::string("PARTIAL")),
                ("analyzed_sources", Value::number(source_count)),
                ("gaps", Value::array(gaps)),
            ]),
        ),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]);
    let id = result.sha256();
    result
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(id));
    Ok(result)
}

pub fn human(report: &Value) -> Result<String> {
    const PREVIEW_LIMIT: usize = 10;
    let effects = report.get("effects")?.array_ref()?;
    let gaps = report.get("unknown")?.array_ref()?.len();
    let mut lines = vec![
        format!(
            "Excession change impact: {} candidate dependent paths.",
            effects.len()
        ),
        format!("Base: {}", report.get("base_commit")?.str()?),
        format!("Head: {}", report.get("head_commit")?.str()?),
    ];
    if effects.is_empty() {
        lines.push("No candidate dependents found in the analyzed scope.".to_owned());
    } else {
        lines.push("Candidate paths (source -> changed dependency):".to_owned());
        for effect in effects.iter().take(PREVIEW_LIMIT) {
            let hops = effect.get("hops")?.array_ref()?;
            let last = hops.last().ok_or("impact candidate has no evidence path")?;
            lines.push(format!(
                "  {:?} -> {:?} ({} hop{})",
                effect.get("entity")?.get("path")?.str()?,
                last.get("target")?.get("path")?.str()?,
                hops.len(),
                if hops.len() == 1 { "" } else { "s" }
            ));
            lines.push(format!("    claim {}", effect.get("claim_id")?.str()?));
        }
        if effects.len() > PREVIEW_LIMIT {
            lines.push(format!(
                "  ... {} additional candidate paths (use --json to inspect all)",
                effects.len() - PREVIEW_LIMIT
            ));
        }
    }
    lines.push(format!("Unknown/unsupported: {gaps} observations."));
    lines.push("Coverage: PARTIAL. Unlisted tests cannot be assumed safe to skip.".to_owned());
    lines.push(
        "Use --output ABSOLUTE_PATH to save a replayable JSON report; --json prints full evidence."
            .to_owned(),
    );
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_promoting_static_candidates_into_observed_proof() {
        let report = obj(&[(
            "relationships",
            Value::array([obj(&[
                ("kind", Value::string("STATIC_IMPORT")),
                ("source", obj(&[("path", Value::string("src/a.py"))])),
                ("target", obj(&[("path", Value::string("src/b.py"))])),
                (
                    "evidence",
                    Value::array([obj(&[("assessment", Value::string("OBSERVED"))])]),
                ),
            ])]),
        )]);
        assert!(incoming(&report).is_err());
    }

    #[test]
    fn preserves_independent_paths_through_a_diamond() {
        let make_edge = |source: &str, target: &str| {
            obj(&[
                ("kind", Value::string("STATIC_IMPORT")),
                ("source", obj(&[("path", Value::string(source))])),
                ("target", obj(&[("path", Value::string(target))])),
                (
                    "evidence",
                    Value::array([obj(&[("assessment", Value::string("CANDIDATE"))])]),
                ),
            ])
        };
        let input = obj(&[(
            "relationships",
            Value::array([
                make_edge("pkg/left.py", "pkg/changed.py"),
                make_edge("pkg/right.py", "pkg/changed.py"),
                make_edge("pkg/top.py", "pkg/left.py"),
                make_edge("pkg/top.py", "pkg/right.py"),
            ]),
        )]);
        let mut effects = Vec::new();
        assert!(
            !walk(
                &incoming(&input).unwrap(),
                &BTreeSet::from(["pkg/changed.py".to_owned()]),
                &mut effects,
            )
            .unwrap()
        );
        let from_top = effects
            .iter()
            .filter(|effect| {
                effect
                    .get("entity")
                    .unwrap()
                    .get("path")
                    .unwrap()
                    .str()
                    .unwrap()
                    == "pkg/top.py"
            })
            .collect::<Vec<_>>();
        assert_eq!(from_top.len(), 2, "each distinct import path has evidence");
        assert_ne!(
            from_top[0].get("claim_id").unwrap(),
            from_top[1].get("claim_id").unwrap()
        );
    }

    #[test]
    fn traverses_reverse_import_with_original_evidence() {
        let input = obj(&[(
            "relationships",
            Value::array([obj(&[
                ("kind", Value::string("STATIC_IMPORT")),
                ("source", obj(&[("path", Value::string("pkg/first.py"))])),
                ("target", obj(&[("path", Value::string("pkg/second.py"))])),
                (
                    "evidence",
                    Value::array([obj(&[
                        ("assessment", Value::string("CANDIDATE")),
                        ("receipt_id", Value::string("receipt")),
                    ])]),
                ),
            ])]),
        )]);
        let mut effects = Vec::new();
        assert!(
            !walk(
                &incoming(&input).unwrap(),
                &BTreeSet::from(["pkg/second.py".to_owned()]),
                &mut effects
            )
            .unwrap()
        );
        assert_eq!(effects.len(), 1);
        assert_eq!(
            effects[0].get("assessment").unwrap().str().unwrap(),
            "CANDIDATE_PATH"
        );
        assert_eq!(
            effects[0].get("hops").unwrap().array_ref().unwrap()[0]
                .get("evidence")
                .unwrap()
                .array_ref()
                .unwrap()[0]
                .get("receipt_id")
                .unwrap()
                .str()
                .unwrap(),
            "receipt"
        );
    }
}
