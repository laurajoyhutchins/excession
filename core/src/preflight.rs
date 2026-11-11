//! Cheap, offline shape and content-digest validation before expensive Git replay.
//! A successful preflight is NOT source authentication, syntax completeness,
//! or a dependency guarantee. Only full audit replays exact Git blobs.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SOURCES: usize = 500;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_OBSERVATIONS: usize = 2048;
const MAX_RELATIONS: usize = 10_000;

fn keys(value: &Value, expected: &[&str]) -> Result<()> {
    let fields = value.entries()?;
    if fields.len() != expected.len() || expected.iter().any(|k| !fields.contains_key(*k)) {
        return refuse("preflight: unexpected or missing report field");
    }
    Ok(())
}

fn bounded_string<'a>(value: &'a Value, limit: usize) -> Result<&'a str> {
    let s = value.str()?;
    if s.is_empty() || s.len() > limit {
        return refuse("preflight: empty or oversized string");
    }
    Ok(s)
}

fn positive_line(value: &Value) -> Result<()> {
    let line = value.integer()?;
    if line < 1 || line > MAX_SOURCE_BYTES as i64 + 1 {
        return refuse("preflight: invalid line number");
    }
    Ok(())
}

fn object_id(value: &Value) -> Result<()> {
    git::oid(value.str()?)?;
    Ok(())
}

fn source_path(value: &Value) -> Result<()> {
    git::validate_path(value.str()?)?;
    Ok(())
}

fn digest(value: &Value, id: &str) -> Result<String> {
    let mut payload = value.entries()?.clone();
    let claimed = payload
        .remove(id)
        .ok_or("preflight: missing report digest")?;
    let asserted = bounded_string(&claimed, 64)?;
    // Compare against the canonical body, excluding the asserted digest.
    let calculated = Value::Object(payload).sha256();
    if asserted != calculated {
        return refuse("preflight: canonical report digest mismatch");
    }
    Ok(calculated)
}

fn status(source: &Value) -> Result<bool> {
    let state = source.get("status")?.str()?;
    if !matches!(
        state,
        "PARSED"
            | "PARSED_WITH_ERRORS"
            | "UNSUPPORTED_LANGUAGE"
            | "GRAMMAR_NOT_INSTALLED"
            | "PARSER_ERROR"
            | "RESOURCE_LIMIT"
    ) {
        return refuse("preflight: unknown syntax status");
    }
    let language = source.get("language")?.str()?;
    if language.len() > 64 || language.is_empty() != (state == "UNSUPPORTED_LANGUAGE") {
        return refuse("preflight: invalid language/status pairing");
    }
    Ok(matches!(state, "PARSED" | "PARSED_WITH_ERRORS"))
}

fn structure(node: &Value, depth: usize, count: &mut usize) -> Result<()> {
    if depth > 64 || *count >= MAX_OBSERVATIONS {
        return refuse("preflight: structure exceeds depth or observation budget");
    }
    *count += 1;
    keys(
        node,
        &["kind", "name", "line", "start_byte", "end_byte", "children"],
    )?;
    bounded_string(node.get("kind")?, 128)?;
    if node.get("name")?.str()?.len() > 1024 {
        return refuse("preflight: oversized symbol name");
    }
    positive_line(node.get("line")?)?;
    let first = node.get("start_byte")?.integer()?;
    let last = node.get("end_byte")?.integer()?;
    if first < 0 || last <= first || last > MAX_SOURCE_BYTES as i64 {
        return refuse("preflight: invalid syntax byte span");
    }
    for child in node.get("children")?.array_ref()? {
        structure(child, depth + 1, count)?;
    }
    Ok(())
}

fn source_record(source: &Value, roots: &BTreeSet<String>) -> Result<()> {
    keys(
        source,
        &[
            "source_path",
            "source_blob",
            "source_mode",
            "language",
            "status",
            "structure",
            "imports",
            "diagnostics",
        ],
    )?;
    let path = source.get("source_path")?.str()?;
    git::validate_path(path)?;
    if !roots
        .iter()
        .any(|r| path == r || path.starts_with(&format!("{r}/")))
    {
        return refuse("preflight: source outside declared roots");
    }
    object_id(source.get("source_blob")?)?;
    if !matches!(source.get("source_mode")?.str()?, "100644" | "100755") {
        return refuse("preflight: not a regular Git source");
    }
    let parsed = status(source)?;
    let mut count = 0;
    for node in source.get("structure")?.array_ref()? {
        structure(node, 0, &mut count)?;
    }
    for imp in source.get("imports")?.array_ref()? {
        if count >= MAX_OBSERVATIONS {
            return refuse("preflight: source observation budget exceeded");
        }
        count += 1;
        keys(imp, &["source", "line", "wildcard"])?;
        if imp.get("source")?.str()?.len() > 4096 {
            return refuse("preflight: oversized import statement");
        }
        positive_line(imp.get("line")?)?;
        imp.get("wildcard")?.bool()?;
    }
    for note in source.get("diagnostics")?.array_ref()? {
        if count >= MAX_OBSERVATIONS {
            return refuse("preflight: source observation budget exceeded");
        }
        count += 1;
        keys(note, &["line", "message"])?;
        positive_line(note.get("line")?)?;
        if note.get("message")?.str()?.len() > 2048 {
            return refuse("preflight: oversized diagnostic");
        }
    }
    if !parsed && count != 0 {
        return refuse("preflight: failed parser supplied syntax observations");
    }
    Ok(())
}

pub fn syntax(value: &Value) -> Result<Value> {
    keys(
        value,
        &[
            "schema",
            "parser",
            "parser_version",
            "completeness",
            "source_commit",
            "source_tree",
            "roots",
            "source_count",
            "sources",
            "report_id",
        ],
    )?;
    if value.get("schema")?.str()? != "excession.syntax-observations.v1"
        || value.get("parser")?.str()? != "tree-sitter-language-pack"
        || value.get("completeness")?.str()? != "CONSERVATIVE_SYNTAX_ONLY"
    {
        return refuse("preflight: incompatible syntax report contract");
    }
    bounded_string(value.get("parser_version")?, 64)?;
    object_id(value.get("source_commit")?)?;
    object_id(value.get("source_tree")?)?;
    let declared = value.get("source_count")?.integer()?;
    let sources = value.get("sources")?.array_ref()?;
    if sources.is_empty() || sources.len() > MAX_SOURCES || declared != sources.len() as i64 {
        return refuse("preflight: inconsistent syntax source count");
    }
    let roots = value.get("roots")?.array_ref()?;
    if roots.is_empty() || roots.len() > 32 {
        return refuse("preflight: invalid root count");
    }
    let mut root_set = BTreeSet::new();
    for root in roots {
        let path = root.str()?;
        git::validate_path(path)?;
        root_set.insert(path.to_owned());
    }
    let mut previous = "";
    for source in sources {
        source_record(source, &root_set)?;
        let path = source.get("source_path")?.str()?;
        if path <= previous {
            return refuse("preflight: sources are duplicated or not sorted");
        }
        previous = path;
    }
    let id = digest(value, "report_id")?;
    Ok(obj(&[
        ("report_id", Value::string(id)),
        ("source_count", Value::number(sources.len())),
        ("status", Value::string("SHAPE_AND_DIGEST_ONLY")),
    ]))
}

fn candidate_source(value: &Value) -> Result<()> {
    source_path(value.get("source_path")?)?;
    object_id(value.get("source_blob")?)?;
    bounded_string(value.get("language")?, 64)?;
    bounded_string(value.get("specifier")?, 4096)?;
    positive_line(value.get("line")?)?;
    Ok(())
}

pub fn resolution(value: &Value) -> Result<Value> {
    keys(
        value,
        &[
            "schema",
            "source_commit",
            "source_tree",
            "syntax_report_id",
            "completeness",
            "candidate_count",
            "unresolved_count",
            "candidate_edges",
            "unresolved_imports",
            "skipped_sources",
            "metadata",
            "resolution_id",
        ],
    )?;
    if value.get("schema")?.str()? != "excession.syntax-dependencies.v1"
        || value.get("completeness")?.str()? != "CONSERVATIVE_CANDIDATES_ONLY"
    {
        return refuse("preflight: incompatible resolution report contract");
    }
    object_id(value.get("source_commit")?)?;
    object_id(value.get("source_tree")?)?;
    bounded_string(value.get("syntax_report_id")?, 64)?;
    let edges = value.get("candidate_edges")?.array_ref()?;
    let unresolved = value.get("unresolved_imports")?.array_ref()?;
    if edges.len() + unresolved.len() > MAX_RELATIONS
        || value.get("candidate_count")?.integer()? != edges.len() as i64
        || value.get("unresolved_count")?.integer()? != unresolved.len() as i64
    {
        return refuse("preflight: inconsistent resolution counts");
    }
    for edge in edges {
        keys(
            edge,
            &[
                "source_path",
                "source_blob",
                "target_path",
                "target_blob",
                "language",
                "specifier",
                "line",
                "resolution",
            ],
        )?;
        candidate_source(edge)?;
        source_path(edge.get("target_path")?)?;
        object_id(edge.get("target_blob")?)?;
        if !matches!(
            edge.get("resolution")?.str()?,
            "RELATIVE_PATH_CANDIDATE"
                | "PYTHON_MODULE_CANDIDATE"
                | "RUST_CRATE_PATH_CANDIDATE"
                | "GO_MODULE_PACKAGE_CANDIDATE"
        ) {
            return refuse("preflight: unknown candidate resolution type");
        }
    }
    for item in unresolved {
        keys(
            item,
            &[
                "source_path",
                "source_blob",
                "language",
                "specifier",
                "line",
                "reason",
            ],
        )?;
        candidate_source(item)?;
        bounded_string(item.get("reason")?, 128)?;
    }
    let skipped = value.get("skipped_sources")?.array_ref()?;
    if skipped.len() > MAX_SOURCES {
        return refuse("preflight: too many skipped sources");
    }
    for item in skipped {
        keys(item, &["source_path", "status"])?;
        source_path(item.get("source_path")?)?;
        bounded_string(item.get("status")?, 64)?;
    }
    let metadata = value.get("metadata")?.array_ref()?;
    if metadata.len() > 32 {
        return refuse("preflight: too many module declarations");
    }
    for item in metadata {
        keys(item, &["source_path", "source_blob", "module"])?;
        source_path(item.get("source_path")?)?;
        object_id(item.get("source_blob")?)?;
        bounded_string(item.get("module")?, 4096)?;
    }
    let id = digest(value, "resolution_id")?;
    Ok(obj(&[
        ("resolution_id", Value::string(id)),
        ("candidate_count", Value::number(edges.len())),
        ("status", Value::string("SHAPE_AND_DIGEST_ONLY")),
    ]))
}

fn read(path: &Path) -> Result<Value> {
    let data = std::fs::read(path).map_err(|e| format!("preflight report read: {e}"))?;
    if data.len() > MAX_REPORT_BYTES {
        return refuse("preflight: report exceeds 32 MiB");
    }
    json::parse(&data)
}

pub fn syntax_file(path: &Path) -> Result<Value> {
    syntax(&read(path)?)
}

pub fn resolution_file(path: &Path) -> Result<Value> {
    resolution(&read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_syntax() -> Value {
        let source = obj(&[
            ("source_path", Value::string("src/a.rs")),
            ("source_blob", Value::string("a".repeat(40))),
            ("source_mode", Value::string("100644")),
            ("language", Value::string("rust")),
            ("status", Value::string("PARSED")),
            ("structure", Value::array([])),
            ("imports", Value::array([])),
            ("diagnostics", Value::array([])),
        ]);
        let payload = obj(&[
            ("schema", Value::string("excession.syntax-observations.v1")),
            ("parser", Value::string("tree-sitter-language-pack")),
            ("parser_version", Value::string("1.25.0")),
            ("completeness", Value::string("CONSERVATIVE_SYNTAX_ONLY")),
            ("source_commit", Value::string("a".repeat(40))),
            ("source_tree", Value::string("b".repeat(40))),
            ("roots", Value::array([Value::string("src")])),
            ("source_count", Value::number(1)),
            ("sources", Value::array([source])),
        ]);
        let mut map = payload.entries().unwrap().clone();
        map.insert("report_id".into(), Value::string(payload.sha256()));
        Value::Object(map)
    }

    #[test]
    fn valid_shape_never_claims_git_authentication() {
        let report = valid_syntax();
        let result = syntax(&report).unwrap();
        assert_eq!(
            result.get("status").unwrap().str().unwrap(),
            "SHAPE_AND_DIGEST_ONLY"
        );
    }

    #[test]
    fn stale_digest_or_extra_field_refused_before_parsing() {
        let mut report = valid_syntax();
        report
            .entries_mut()
            .unwrap()
            .insert("report_id".into(), Value::string("0".repeat(64)));
        assert!(syntax(&report).is_err());
        let mut report = valid_syntax();
        report
            .entries_mut()
            .unwrap()
            .insert("unexpected".into(), Value::string("extra"));
        assert!(syntax(&report).is_err());
    }

    #[test]
    fn self_consistent_forgery_is_only_provisionally_valid() {
        let mut report = valid_syntax();
        let mut items = report.get("sources").unwrap().array_ref().unwrap().to_vec();
        items[0]
            .entries_mut()
            .unwrap()
            .insert("source_blob".into(), Value::string("f".repeat(40)));
        report
            .entries_mut()
            .unwrap()
            .insert("sources".into(), Value::array(items));
        let mut payload = report.entries().unwrap().clone();
        payload.remove("report_id");
        let id = Value::Object(payload).sha256();
        report
            .entries_mut()
            .unwrap()
            .insert("report_id".into(), Value::string(id));
        assert!(syntax(&report).is_ok()); // Only Git replay can reject the lie.
    }

    #[test]
    fn malformed_source_and_observations_refused() {
        let mut report = valid_syntax();
        let mut items = report.get("sources").unwrap().array_ref().unwrap().to_vec();
        items[0]
            .entries_mut()
            .unwrap()
            .insert("source_path".into(), Value::string("../escape"));
        report
            .entries_mut()
            .unwrap()
            .insert("sources".into(), Value::array(items));
        assert!(syntax(&report).is_err());
    }
}
