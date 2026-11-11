//! Read-only, bounded Linux-source extraction; Git authenticates every input.
//! Facts describe syntax, not evaluated build configurations or compiled C.
use crate::json::{Value, obj};
use crate::{Result, adapter, git, refuse};
use std::path::Path;

const MAX_FILES: usize = 1_500;
const MAX_FILE_BYTES: usize = 1_048_576;
const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 100_000;
const BATCH: usize = 64;

fn eligible(path: &str) -> bool {
    path.ends_with(".c")
        || path.ends_with(".h")
        || path.rsplit('/').next().is_some_and(|name| {
            name == "Makefile" || name == "Kbuild" || name.starts_with("Kconfig")
        })
}

fn within(path: &str, roots: &[&str]) -> bool {
    roots.iter().any(|root| path == *root || path.starts_with(&format!("{root}/")))
}

fn checked_fact(f: &Value, max_line: usize) -> Result<()> {
    let line = f.get("line")?.integer()?;
    if line < 1 || usize::try_from(line).map_err(|_| "negative fact line")? > max_line {
        return refuse("kernel fact line falls outside pinned source");
    }
    let kind = f.get("kind")?.str()?;
    if !matches!(
        kind,
        "c_include" | "c_export" | "c_macro" | "build_object"
            | "build_include" | "kconfig_symbol" | "kconfig_depends"
            | "kconfig_select" | "kconfig_imply" | "kconfig_default" | "kconfig_source"
    ) {
        return refuse("unknown kernel fact type");
    }
    if !matches!(
        f.get("status")?.str()?,
        "DECLARED" | "LEXICAL_CANDIDATE" | "UNRESOLVED_INCLUDE"
            | "UNRESOLVED_CONFIG" | "UNRESOLVED_SOURCE" | "UNRESOLVED_MAKE"
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
pub fn scan(repo: &Path, revision: &str, roots: &[&str]) -> Result<Value> {
    if roots.is_empty() || roots.len() > 16 {
        return refuse("kernel scan requires 1..16 source roots");
    }
    for root in roots {
        git::validate_path(root)?;
    }
    git::clean(repo)?;
    let commit = git::rev(repo, revision)?;
    if git::text(repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("kernel scan revision must resolve to a commit");
    }
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let source = git::entries(repo, &commit)?;
    let selected: Vec<_> = source
        .into_iter()
        .filter(|e| {
            e.kind == "blob"
                && matches!(e.mode.as_str(), "100644" | "100755")
                && eligible(&e.path)
                && within(&e.path, roots)
        })
        .collect();
    if selected.is_empty() || selected.len() > MAX_FILES {
        return refuse(format!("kernel scan needs 1..{MAX_FILES} eligible tracked files"));
    }
    let mut reader = git::BlobReader::open(repo)?;
    let mut results = Vec::with_capacity(selected.len());
    let mut bytes_seen = 0usize;
    let mut fact_count = 0usize;
    let mut diagnostic_count = 0usize;
    for chunk in selected.chunks(BATCH) {
        let mut inputs = Vec::with_capacity(chunk.len());
        let mut line_counts = Vec::with_capacity(chunk.len());
        for entry in chunk {
            let bytes = reader.read(&entry.oid)?;
            bytes_seen = bytes_seen.checked_add(bytes.len()).ok_or("kernel source size overflow")?;
            if bytes.len() > MAX_FILE_BYTES || bytes_seen > MAX_INPUT_BYTES {
                return refuse("kernel scan exceeded source byte budget");
            }
            let content = String::from_utf8(bytes)
                .map_err(|_| format!("non-UTF8 source unsupported: {}", entry.path))?;
            line_counts.push(content.lines().count().max(1));
            inputs.push(obj(&[
                ("path", Value::string(&entry.path)),
                ("content", Value::string(content)),
            ]));
        }
        let output = adapter::kernel_sources(&inputs)?;
        let parsed = output.array_ref()?;
        if parsed.len() != chunk.len() {
            return refuse("kernel parser changed source record count");
        }
        for ((item, entry), line_count) in parsed.iter().zip(chunk).zip(line_counts) {
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
                if value.entries()?.len() != 2 || line < 1
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
    let body = obj(&[
        ("schema", Value::string("excession.kernel-syntax.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("roots", Value::array(roots.iter().map(|s| Value::string(*s)))),
        ("source_count", Value::number(results.len())),
        ("fact_count", Value::number(fact_count)),
        ("diagnostic_count", Value::number(diagnostic_count)),
        ("source_bytes", Value::number(bytes_seen)),
        ("sources", Value::array(results)),
        ("completeness", Value::string("CONSERVATIVE_SYNTAX_ONLY")),
    ]);
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
        wrong.entries_mut().unwrap().insert("status".into(), Value::string("VERIFIED"));
        assert!(checked_fact(&wrong, 5).is_err());
    }
}
