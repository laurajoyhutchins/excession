//! Read-only, bounded Linux-source extraction; Git authenticates every input.
//! Facts describe syntax, not evaluated build configurations or compiled C.
use crate::json::{Value, obj};
use crate::{Result, adapter, digest, git, kernel_links, refuse};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_FILES: usize = 1_500;
const MAX_FILE_BYTES: usize = 1_048_576;
const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 100_000;
const BATCH: usize = 64;
pub const MAX_PARTITION_FILES: usize = 128;

pub(crate) fn eligible(path: &str) -> bool {
    path.ends_with(".c")
        || path.ends_with(".h")
        || path.rsplit('/').next().is_some_and(|name| {
            name == "Makefile" || name == "Kbuild" || name.starts_with("Kconfig")
        })
}

fn within(path: &str, roots: &[&str]) -> bool {
    roots
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
}

fn checked_fact(f: &Value, max_line: usize) -> Result<()> {
    let line = f.get("line")?.integer()?;
    if line < 1 || usize::try_from(line).map_err(|_| "negative fact line")? > max_line {
        return refuse("kernel fact line falls outside pinned source");
    }
    let kind = f.get("kind")?.str()?;
    if !matches!(
        kind,
        "c_include"
            | "c_export"
            | "c_macro"
            | "build_object"
            | "build_include"
            | "kconfig_symbol"
            | "kconfig_depends"
            | "kconfig_select"
            | "kconfig_imply"
            | "kconfig_default"
            | "kconfig_source"
    ) {
        return refuse("unknown kernel fact type");
    }
    if !matches!(
        f.get("status")?.str()?,
        "DECLARED"
            | "LEXICAL_CANDIDATE"
            | "UNRESOLVED_INCLUDE"
            | "UNRESOLVED_CONFIG"
            | "UNRESOLVED_SOURCE"
            | "UNRESOLVED_MAKE"
    ) {
        return refuse("unknown kernel fact status");
    }
    for key in ["subject", "target", "condition"] {
        if f.get(key)?.str()?.len() > 4_096 {
            return refuse("oversized kernel fact");
        }
    }
    if f.entries()?.len() != 6 {
        return refuse("unrecognized kernel fact fields");
    }
    Ok(())
}

/// Parse selected Git blobs without reading the working-tree file contents.
///
/// This scan intentionally does not build Linux, choose a Kconfig assignment,
/// expand GNU Make functions, or claim Clang AST/compilation coverage.
/// Inventory every tracked, regular C/H/Kbuild/Kconfig source in a pinned Git
/// revision without fetching any source blobs (works on sparse checkouts).
pub fn inventory(repo: &Path, revision: &str, partition_size: usize) -> Result<Value> {
    let (commit, tree, selected) = snapshot(repo, revision)?;
    let (digest, total) = inventory_identity(&selected);
    let count = partition_count(total, partition_size)?;
    let body = obj(&[
        ("schema", Value::string("excession.kernel-inventory.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("eligible_source_count", Value::number(total)),
        ("inventory_sha256", Value::string(digest)),
        ("partition_size", Value::number(partition_size)),
        ("partition_count", Value::number(count)),
        ("scope", Value::string("TRACKED_REGULAR_C_H_KBUILD_KCONFIG")),
        ("status", Value::string("GIT_INVENTORY_ONLY_NOT_PARSED")),
    ]);
    let mut result = body.entries()?.clone();
    result.insert("report_id".into(), Value::string(body.sha256()));
    Ok(Value::Object(result))
}

pub(crate) fn snapshot(repo: &Path, revision: &str) -> Result<(String, String, Vec<git::Entry>)> {
    git::verify_repo(repo)?;
    let commit = git::rev(repo, revision)?;
    if git::text(repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("kernel revision must resolve to a commit");
    }
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let mut sources = git::entries(repo, &commit)?
        .into_iter()
        .filter(|e| {
            e.kind == "blob" && matches!(e.mode.as_str(), "100644" | "100755") && eligible(&e.path)
        })
        .collect::<Vec<_>>();
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((commit, tree, sources))
}
pub(crate) fn inventory_identity(entries: &[git::Entry]) -> (String, usize) {
    let mut bytes = Vec::new();
    for e in entries {
        for part in [&e.path, &e.mode, &e.oid] {
            bytes.extend_from_slice(part.as_bytes());
            bytes.push(0);
        }
    }
    (digest::sha256(&bytes), entries.len())
}
pub(crate) fn partition_count(total: usize, size: usize) -> Result<usize> {
    if size == 0 || size > MAX_PARTITION_FILES {
        return refuse(format!(
            "kernel partition size must be 1..{MAX_PARTITION_FILES}"
        ));
    }
    Ok(total.div_ceil(size))
}

/// A source batch is a reproducible, independently replayable slice of the
/// full Git inventory. This is syntax coverage, NOT evaluated Kbuild coverage.
pub fn scan_partition(repo: &Path, revision: &str, index: usize, size: usize) -> Result<Value> {
    partition_count(1, size)?;
    scan_impl(repo, revision, None, Some((index, size)))
}

pub fn scan(repo: &Path, revision: &str, roots: &[&str]) -> Result<Value> {
    if roots.is_empty() || roots.len() > 16 {
        return refuse("kernel scan requires 1..16 source roots");
    }
    for root in roots {
        git::validate_path(root)?;
    }
    scan_impl(repo, revision, Some(roots), None)
}
fn scan_impl(
    repo: &Path,
    revision: &str,
    roots: Option<&[&str]>,
    partition: Option<(usize, usize)>,
) -> Result<Value> {
    git::clean(repo)?;
    let (commit, tree, eligible_sources) = snapshot(repo, revision)?;
    let inventory_sha256 = inventory_identity(&eligible_sources).0;
    let total_eligible = eligible_sources.len();
    let source = git::entries(repo, &commit)?;
    let tracked = source
        .into_iter()
        .filter(|e| e.kind == "blob")
        .map(|e| (e.path, e.oid))
        .collect::<BTreeMap<_, _>>();
    let selected = if let Some((index, size)) = partition {
        let count = partition_count(total_eligible, size)?;
        if index >= count {
            return refuse("kernel partition index outside tracked source inventory");
        }
        let first = index.checked_mul(size).ok_or("partition offset overflow")?;
        let last = first.saturating_add(size).min(total_eligible);
        eligible_sources[first..last].to_vec()
    } else {
        let roots = roots.ok_or("missing selected roots")?;
        eligible_sources
            .into_iter()
            .filter(|e| within(&e.path, roots))
            .collect()
    };
    if selected.is_empty() || selected.len() > MAX_FILES {
        return refuse(format!(
            "kernel scan needs 1..{MAX_FILES} eligible tracked files"
        ));
    }
    let mut reader = git::BlobReader::open(repo)?;
    let mut results = Vec::with_capacity(selected.len());
    let mut bytes_seen = 0usize;
    let mut fact_count = 0usize;
    let mut diagnostic_count = 0usize;
    let mut unsupported = Vec::new();
    for chunk in selected.chunks(BATCH) {
        let mut inputs = Vec::with_capacity(chunk.len());
        let mut line_counts = Vec::with_capacity(chunk.len());
        let mut parsed_entries = Vec::with_capacity(chunk.len());
        for entry in chunk {
            let bytes = reader.read(&entry.oid)?;
            let reason = if bytes.len() > MAX_FILE_BYTES {
                Some("SOURCE_EXCEEDS_1_MIB")
            } else if bytes_seen.saturating_add(bytes.len()) > MAX_INPUT_BYTES {
                Some("PARTITION_EXCEEDS_32_MIB")
            } else if std::str::from_utf8(&bytes).is_err() {
                Some("SOURCE_NOT_UTF8")
            } else {
                None
            };
            if let Some(reason) = reason {
                if partition.is_none() {
                    return refuse(format!(
                        "unsupported kernel source {}: {reason}",
                        entry.path
                    ));
                }
                unsupported.push(obj(&[
                    ("source_path", Value::string(&entry.path)),
                    ("source_blob", Value::string(&entry.oid)),
                    ("reason", Value::string(reason)),
                ]));
                continue;
            }
            bytes_seen += bytes.len();
            let content = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            line_counts.push(content.lines().count().max(1));
            parsed_entries.push(entry);
            inputs.push(obj(&[
                ("path", Value::string(&entry.path)),
                ("content", Value::string(content)),
            ]));
        }
        if inputs.is_empty() {
            continue;
        }
        let output = adapter::kernel_sources(&inputs)?;
        let parsed = output.array_ref()?;
        if parsed.len() != parsed_entries.len() {
            return refuse("kernel parser changed source record count");
        }
        for ((item, entry), line_count) in parsed.iter().zip(parsed_entries).zip(line_counts) {
            if item.entries()?.len() != 3 || item.get("path")?.str()? != entry.path {
                return refuse("kernel parser returned unknown source identity");
            }
            let facts = item.get("facts")?.array_ref()?;
            let diagnostics = item.get("diagnostics")?.array_ref()?;
            for value in facts {
                checked_fact(value, line_count)?;
            }
            for value in diagnostics {
                let line = value.get("line")?.integer()?;
                if value.entries()?.len() != 2
                    || line < 1
                    || usize::try_from(line).map_err(|_| "negative diagnostic line")? > line_count
                    || value.get("reason")?.str()?.len() > 512
                {
                    return refuse("invalid kernel parser diagnostic");
                }
            }
            fact_count += facts.len();
            diagnostic_count += diagnostics.len();
            if fact_count + diagnostic_count > MAX_OBSERVATIONS {
                return refuse("kernel parser exceeded observation budget");
            }
            results.push(obj(&[
                ("source_path", Value::string(&entry.path)),
                ("source_blob", Value::string(&entry.oid)),
                ("source_mode", Value::string(&entry.mode)),
                ("facts", item.get("facts")?.clone()),
                ("diagnostics", item.get("diagnostics")?.clone()),
            ]));
        }
    }
    reader.finish()?;
    let links = kernel_links::derive(&results, &tracked)?;
    if links.len() + fact_count + diagnostic_count > MAX_OBSERVATIONS {
        return refuse("kernel link count exceeded observation budget");
    }
    let mut fields = vec![
        ("schema", Value::string("excession.kernel-syntax.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        (
            "roots",
            Value::array(roots.unwrap_or(&[]).iter().map(|s| Value::string(*s))),
        ),
        ("source_count", Value::number(results.len())),
        ("fact_count", Value::number(fact_count)),
        ("diagnostic_count", Value::number(diagnostic_count)),
        ("link_count", Value::number(links.len())),
        ("links", Value::array(links)),
        ("source_bytes", Value::number(bytes_seen)),
        ("sources", Value::array(results)),
        ("completeness", Value::string("CONSERVATIVE_SYNTAX_ONLY")),
    ];
    if let Some((index, size)) = partition {
        let count = partition_count(total_eligible, size)?;
        fields.extend([
            ("unsupported", Value::array(unsupported)),
            ("selected_source_count", Value::number(selected.len())),
            ("total_eligible_source_count", Value::number(total_eligible)),
            (
                "partition",
                obj(&[
                    ("index", Value::number(index)),
                    ("size", Value::number(size)),
                    ("count", Value::number(count)),
                    ("inventory_sha256", Value::string(&inventory_sha256)),
                ]),
            ),
        ]);
    }
    let body = obj(&fields);
    let id = body.sha256();
    let mut report = body.entries()?.clone();
    report.insert("report_id".into(), Value::string(id));
    Ok(Value::Object(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_scope_is_component_aware() {
        assert!(eligible("drivers/rtc/rtc-cmos.c"));
        assert!(eligible("drivers/rtc/Kconfig"));
        assert!(eligible("drivers/rtc/Makefile"));
        assert!(!eligible("drivers/rtc/README.md"));
        assert!(within("drivers/rtc/rtc-cmos.c", &["drivers/rtc"]));
        assert!(!within("drivers/rtclock/a.c", &["drivers/rtc"]));
    }
    #[test]
    fn rejects_parser_facts_outside_source() {
        let good = obj(&[
            ("line", Value::number(1)),
            ("kind", Value::string("c_include")),
            ("subject", Value::string("quoted")),
            ("target", Value::string("foo.h")),
            ("condition", Value::string("")),
            ("status", Value::string("UNRESOLVED_INCLUDE")),
        ]);
        assert!(checked_fact(&good, 1).is_ok());
        assert!(checked_fact(&good, 0).is_err());
        let mut wrong = good.clone();
        wrong
            .entries_mut()
            .unwrap()
            .insert("status".into(), Value::string("VERIFIED"));
        assert!(checked_fact(&wrong, 5).is_err());
    }
}
