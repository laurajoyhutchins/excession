//! Validate Git-complete, partitioned Linux syntax observations.
//! Coverage means accounting for every eligible source blob, not resolving
//! configuration-dependent build or runtime relations.
use crate::json::{Value, obj};
use crate::{Result, git, kernel, refuse};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const MAX_RECEIPT_BYTES: u64 = 64 * 1024 * 1024;

/// Read independently produced part reports, reject omissions, overlaps and
/// changes of Git revision/inventory, and account for unsupported source blobs.
pub fn audit(repo: &Path, revision: &str, directory: &Path, size: usize) -> Result<Value> {
    git::clean(repo)?;
    let (commit, tree, expected) = kernel::snapshot(repo, revision)?;
    let partitions = kernel::partition_count(expected.len(), size)?;
    let inventory_sha = kernel::inventory_identity(&expected).0;
    let mut report_ids = Vec::with_capacity(partitions);
    let mut observed = 0usize;
    let mut unsupported = 0usize;
    let mut facts = 0usize;
    let mut diagnostics = 0usize;
    let mut reasons = BTreeMap::<String, usize>::new();
    for index in 0..partitions {
        let filename = directory.join(format!("part-{index:05}.json"));
        if fs::metadata(&filename)
            .map_err(|e| format!("missing partition {}: {e}", filename.display()))?
            .len() > MAX_RECEIPT_BYTES
        {
            return refuse("partition receipt exceeds 64 MiB");
        }
        let raw = fs::read(&filename).map_err(|e| format!("read partition: {e}"))?;
        let receipt = crate::json::parse(&raw)?;
        if receipt.get("schema")?.str()? != "excession.kernel-syntax.v1"
            || receipt.get("source_commit")?.str()? != commit
            || receipt.get("source_tree")?.str()? != tree
            || receipt.get("completeness")?.str()? != "CONSERVATIVE_SYNTAX_ONLY"
        {
            return refuse(format!("partition {index} does not bind the pinned Git revision"));
        }
        let id = receipt.get("report_id")?.str()?.to_owned();
        let mut body = receipt.entries()?.clone();
        body.remove("report_id");
        if Value::Object(body).sha256() != id {
            return refuse(format!("partition {index} report digest differs from its body"));
        }
        let scope = receipt.get("partition")?;
        if scope.get("index")?.integer()? != index as i64
            || scope.get("size")?.integer()? != size as i64
            || scope.get("count")?.integer()? != partitions as i64
            || scope.get("inventory_sha256")?.str()? != inventory_sha
        {
            return refuse(format!("partition {index} inventory does not match"));
        }
        let start = index.checked_mul(size).ok_or("partition multiplication overflow")?;
        let end = start.saturating_add(size).min(expected.len());
        let selected = &expected[start..end];
        if receipt.get("selected_source_count")?.integer()? != selected.len() as i64
            || receipt.get("total_eligible_source_count")?.integer()? != expected.len() as i64
        {
            return refuse("partition counts do not match the Git inventory");
        }
        let sources = receipt.get("sources")?.array_ref()?;
        let skipped = receipt.get("unsupported")?.array_ref()?;
        if sources.len() + skipped.len() != selected.len()
            || receipt.get("source_count")?.integer()? != sources.len() as i64
        {
            return refuse("partition failed to account for every selected Git source");
        }
        let expected_files = selected.iter()
            .map(|e| (e.path.as_str(), e.oid.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut actual = BTreeMap::<&str, &str>::new();
        for item in sources {
            let path = item.get("source_path")?.str()?;
            let blob = item.get("source_blob")?.str()?;
            if actual.insert(path, blob).is_some() {
                return refuse("duplicate observed source within partition");
            }
        }
        for item in skipped {
            let path = item.get("source_path")?.str()?;
            let blob = item.get("source_blob")?.str()?;
            let reason = item.get("reason")?.str()?;
            if !matches!(reason, "SOURCE_EXCEEDS_1_MIB" | "PARTITION_EXCEEDS_32_MIB" | "SOURCE_NOT_UTF8") {
                return refuse("unrecognized unsupported-source reason");
            }
            *reasons.entry(reason.to_owned()).or_default() += 1;
            if actual.insert(path, blob).is_some() {
                return refuse("duplicate unsupported/observed source within partition");
            }
        }
        if actual != expected_files {
            return refuse(format!("partition {index} omits or substitutes Git sources"));
        }
        observed += sources.len();
        unsupported += skipped.len();
        facts += usize::try_from(receipt.get("fact_count")?.integer()?)
            .map_err(|_| "negative fact count")?;
        diagnostics += usize::try_from(receipt.get("diagnostic_count")?.integer()?)
            .map_err(|_| "negative diagnostic count")?;
        report_ids.push(Value::string(id));
    }
    // Every eligible source is accounted for exactly once. This is not
    // completeness of transitive Kbuild dependencies or of program behavior.
    let body = obj(&[
        ("schema", Value::string("excession.kernel-coverage.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("inventory_sha256", Value::string(inventory_sha)),
        ("partition_size", Value::number(size)),
        ("partition_count", Value::number(partitions)),
        ("eligible_sources", Value::number(expected.len())),
        ("observed_sources", Value::number(observed)),
        ("unsupported_sources", Value::number(unsupported)),
        ("fact_count", Value::number(facts)),
        ("diagnostic_count", Value::number(diagnostics)),
        ("unsupported_reasons", Value::object(reasons.into_iter()
            .map(|(k, v)| (k, Value::number(v))))),
        ("part_report_ids", Value::array(report_ids)),
        ("status", Value::string(if unsupported == 0 {
            "ALL_ELIGIBLE_SYNTAX_OBSERVED"
        } else {
            "ALL_ELIGIBLE_SOURCES_ACCOUNTED_PARTIAL_SYNTAX"
        })),
        ("semantic_scope", Value::string(
            "LEXICAL_C_KBUILD_KCONFIG_FACTS_NOT_CONFIGURED_BUILD_CLOSURE"
        )),
    ]);
    let mut result = body.entries()?.clone();
    result.insert("report_id".into(), Value::string(body.sha256()));
    Ok(Value::Object(result))
}
