//! Source-bound SCIP observations, decoded by the official external \`scip print --json\`.
//! No protobuf reimplementation, source rewriting, or invented FFI relationships.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 1500;
const MAX_OCCURRENCES: usize = 100_000;
const MAX_BLOB_BYTES: usize = 512 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CANDIDATES: usize = 128;
const MAX_REFERENCES_PER_CANDIDATE: usize = 100;

#[derive(Clone)]
struct Document {
    path: String,
    language: String,
    blob: String,
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
struct Reference {
    from: String,
    to: String,
    symbol: String,
    line: usize,
}

fn opt<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.entries().ok()?.get(key)
}

fn property<'a>(value: &'a Value, a: &str, b: &str) -> Option<&'a Value> {
    opt(value, a).or_else(|| opt(value, b))
}

fn property_str<'a>(value: &'a Value, a: &str, b: &str) -> Result<&'a str> {
    property(value, a, b)
        .ok_or_else(|| format!("SCIP missing field {a}"))?
        .str()
}

fn optional_array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    match opt(value, key) {
        None => Ok(&[]),
        Some(value) => value.array_ref(),
    }
}

fn num(value: &Value) -> Result<usize> {
    usize::try_from(value.integer()?).map_err(|_| "negative SCIP offset".to_owned())
}

fn optional_num(value: &Value, a: &str, b: &str) -> Result<usize> {
    match property(value, a, b) {
        Some(v) => num(v),
        None => Ok(0),
    }
}

fn encoding(doc: &Value) -> Result<usize> {
    match property(doc, "position_encoding", "positionEncoding") {
        None => Ok(0),
        Some(Value::Number(v)) => match v.as_str() {
            "0" | "1" | "2" | "3" => Ok(v.parse::<usize>().expect("validated enum")),
            _ => refuse("unknown SCIP position encoding"),
        },
        Some(Value::String(v)) => match v.as_str() {
            "UnspecifiedPositionEncoding" => Ok(0),
            "UTF8CodeUnitOffsetFromLineStart" => Ok(1),
            "UTF16CodeUnitOffsetFromLineStart" => Ok(2),
            "UTF32CodeUnitOffsetFromLineStart" => Ok(3),
            _ => refuse("unknown SCIP position encoding"),
        },
        _ => refuse("invalid SCIP position encoding"),
    }
}

fn source_line_lengths(data: &[u8], encoding: usize) -> Result<Vec<usize>> {
    let source = std::str::from_utf8(data).map_err(|_| "SCIP source is not UTF-8")?;
    Ok(source
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            match encoding {
                1 => line.len(),
                2 => line.encode_utf16().count(),
                3 => line.chars().count(),
                _ => 0,
            }
        })
        .collect())
}

/// Return 0-based starting line after validating both endpoints. For legacy
/// ranges and the generated Go JSON's TypedRange oneof wrappers.
fn range_line(occ: &Value, lines: &[usize], encoding: usize) -> Result<usize> {
    let raw = opt(occ, "range");
    let typed = property(occ, "TypedRange", "typed_range").unwrap_or(occ);
    let single = property(typed, "SingleLineRange", "single_line_range")
        .or_else(|| opt(typed, "singleLineRange"));
    let multi = property(typed, "MultiLineRange", "multi_line_range")
        .or_else(|| opt(typed, "multiLineRange"));
    if single.is_some() && multi.is_some() {
        return refuse("SCIP contains two incompatible typed ranges");
    }
    let (start_line, start_char, end_line, end_char) = if let Some(v) = single {
        let line = optional_num(v, "line", "line")?;
        (
            line,
            optional_num(v, "start_character", "startCharacter")?,
            line,
            optional_num(v, "end_character", "endCharacter")?,
        )
    } else if let Some(v) = multi {
        (
            optional_num(v, "start_line", "startLine")?,
            optional_num(v, "start_character", "startCharacter")?,
            optional_num(v, "end_line", "endLine")?,
            optional_num(v, "end_character", "endCharacter")?,
        )
    } else if let Some(v) = raw {
        let values = v.array_ref()?;
        if !matches!(values.len(), 3 | 4) {
            return refuse("SCIP occurrence requires a 3- or 4-field range");
        }
        let a = num(&values[0])?;
        let b = num(&values[1])?;
        if values.len() == 3 {
            (a, b, a, num(&values[2])?)
        } else {
            (a, b, num(&values[2])?, num(&values[3])?)
        }
    } else {
        return refuse("SCIP symbol occurrence has no supported source range");
    };
    if start_line >= lines.len()
        || end_line >= lines.len()
        || (end_line, end_char) < (start_line, start_char)
    {
        return refuse("SCIP occurrence line/range falls outside pinned Git source");
    }
    // Older SCIP indexes may omit position encoding. Validate lines but do not
    // claim character-level validation for such documents.
    if encoding != 0 && (start_char > lines[start_line] || end_char > lines[end_line]) {
        return refuse("SCIP occurrence column falls outside pinned Git source");
    }
    Ok(start_line)
}

fn candidate(mut value: Value) -> Result<Value> {
    let id = value.sha256();
    value
        .entries_mut()?
        .insert("candidate_id".into(), Value::string(id));
    Ok(value)
}

fn source(document: &Document) -> Value {
    obj(&[
        ("path", Value::string(&document.path)),
        ("blob", Value::string(&document.blob)),
        ("language", Value::string(&document.language)),
    ])
}

fn reference(v: &Reference, documents: &BTreeMap<String, Document>) -> Value {
    obj(&[
        ("from", source(&documents[&v.from])),
        ("to", source(&documents[&v.to])),
        ("symbol", Value::string(&v.symbol)),
        ("line", Value::number(v.line + 1)),
        ("relation", Value::string("SCIP_SYMBOL_REFERENCE")),
    ])
}

/// Recognize one narrowly defined low-signal SCIP shape, without parsing Rust
/// or treating a path layout as evidence of behavior. This is only advisory
/// triage: all native references and source identities remain in the report.
fn crate_root_sibling_module<'a>(root: &str, child: &'a str) -> Option<&'a str> {
    let (directory, filename) = root.rsplit_once('/')?;
    if !(directory == "src" || directory.ends_with("/src"))
        || !matches!(filename, "lib.rs" | "main.rs")
    {
        return None;
    }
    let sibling = child.strip_prefix(directory)?.strip_prefix('/')?;
    if sibling.contains('/') {
        return None;
    }
    let module = sibling.strip_suffix(".rs")?;
    if module.is_empty() || matches!(module, "lib" | "main" | "mod") {
        return None;
    }
    Some(module)
}

fn likely_rust_module_structure(
    indexer: &str,
    left: &Document,
    right: &Document,
    forward: &[Reference],
    backward: &[Reference],
) -> bool {
    if indexer != "rust-analyzer" || left.language != "rust" || right.language != "rust" {
        return false;
    }
    let (module, root_to_child) =
        if let Some(module) = crate_root_sibling_module(&left.path, &right.path) {
            (module, forward)
        } else if let Some(module) = crate_root_sibling_module(&right.path, &left.path) {
            (module, backward)
        } else {
            return false;
        };
    // The native symbol must be exactly the sibling module's symbol.
    // If the crate root also refers to a function/type inside that module,
    // retain the pair among the review candidates. Unindexed links are unknown.
    root_to_child.len() == 1
        && root_to_child[0].symbol.starts_with("rust-analyzer cargo ")
        && root_to_child[0]
            .symbol
            .ends_with(&format!(" {module}/"))
}

/// Accept JSON decoded by the official SCIP CLI. No SCIP indexer is invoked:
/// builds and index generation remain explicit, isolated preparatory work.
pub fn suggest(repo: &Path, revision: &str, input: &Path) -> Result<Value> {
    suggest_with_prefix(repo, revision, input, None)
}

/// Source-prefix binds a nested Cargo project (such as core/) to the outer
/// repository's Git tree, without rewriting the upstream SCIP index.
pub fn suggest_with_prefix(
    repo: &Path,
    revision: &str,
    input: &Path,
    source_prefix: Option<&str>,
) -> Result<Value> {
    let bytes = std::fs::read(input).map_err(|e| format!("SCIP JSON read: {e}"))?;
    analyze_bytes(repo, revision, &bytes, source_prefix, false)
}

/// Export all matched cross-file references, not only refactor candidates.
/// The producer's partial scope and unresolved count remain visible.
pub fn evidence_from_bytes(
    repo: &Path,
    revision: &str,
    input: &[u8],
    source_prefix: Option<&str>,
) -> Result<Value> {
    analyze_bytes(repo, revision, input, source_prefix, true)
}

fn analyze_bytes(
    repo: &Path,
    revision: &str,
    bytes: &[u8],
    source_prefix: Option<&str>,
    include_references: bool,
) -> Result<Value> {
    if let Some(prefix) = source_prefix {
        git::validate_path(prefix)?;
    }
    git::verify_repo(repo)?;
    let commit = git::rev(repo, revision)?;
    if git::text(repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("SCIP revision must be a commit");
    }
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return refuse("SCIP JSON exceeds 16 MiB, narrow the source index");
    }
    let parsed = json::parse(bytes)?;
    let metadata = parsed.get("metadata")?;
    if let Some(prefix) = source_prefix {
        let expected_root = format!(
            "file://{}",
            repo.canonicalize()
                .map_err(|e| format!("SCIP checkout root: {e}"))?
                .join(prefix)
                .display()
        );
        let actual_root = property(metadata, "project_root", "projectRoot")
            .ok_or("SCIP nested index lacks project_root")?
            .str()?;
        if actual_root != expected_root {
            return refuse("SCIP project_root does not match pinned source prefix");
        }
    }
    let tool =
        property(metadata, "tool_info", "toolInfo").ok_or("SCIP metadata lacks tool_info")?;
    let name = property_str(tool, "name", "name")?;
    let version = property_str(tool, "version", "version")?;
    if name.is_empty() || version.is_empty() || name.len() > 128 || version.len() > 128 {
        return refuse("SCIP indexer name/version required");
    }
    let docs = parsed.get("documents")?.array_ref()?;
    if docs.len() > MAX_DOCUMENTS {
        return refuse("SCIP document limit exceeded");
    }

    let tracked = git::entries(repo, &commit)?
        .into_iter()
        .filter(|e| e.kind == "blob" && matches!(e.mode.as_str(), "100644" | "100755"))
        .map(|e| (e.path.clone(), e))
        .collect::<BTreeMap<_, _>>();
    let mut reader = git::BlobReader::open(repo)?;
    let mut documents = BTreeMap::new();
    let mut definitions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut references = Vec::new();
    let mut total_bytes = 0_usize;
    let mut count = 0_usize;
    let mut languages = BTreeSet::new();
    let mut lines_only = 0_usize;

    for doc in docs {
        let relative_path = property_str(doc, "relative_path", "relativePath")?;
        git::validate_path(relative_path)?;
        let expanded;
        let path = if let Some(prefix) = source_prefix {
            expanded = format!("{prefix}/{relative_path}");
            git::validate_path(&expanded)?;
            expanded.as_str()
        } else {
            relative_path
        };
        let entry = tracked
            .get(path)
            .ok_or_else(|| format!("SCIP document not a tracked regular Git file: {path}"))?;
        // Upstream SCIP may omit an unspecified Document.language. Keep
        // unknown explicit rather than fabricating a language identity.
        let language = opt(doc, "language")
            .map(Value::str)
            .transpose()?
            .filter(|language| !language.is_empty())
            .unwrap_or("unspecified");
        if language.len() > 64 {
            return refuse("SCIP document language exceeds 64 bytes");
        }
        if documents.contains_key(path) {
            return refuse("duplicate SCIP source document path");
        }
        let data = reader.read_capped(&entry.oid, MAX_BLOB_BYTES)?;
        total_bytes = total_bytes
            .checked_add(data.len())
            .ok_or("SCIP source byte total overflow")?;
        if total_bytes > MAX_SOURCE_BYTES {
            return refuse("SCIP source bytes exceed 32 MiB");
        }
        if let Some(text) = opt(doc, "text")
            && !text.str()?.is_empty()
            && text.str()?.as_bytes() != data.as_slice()
        {
            return refuse("SCIP embedded document text differs from pinned Git blob");
        }
        let charset = encoding(doc)?;
        if charset == 0 {
            lines_only += 1;
        }
        let lines = source_line_lengths(&data, charset)?;
        languages.insert(language.to_owned());
        documents.insert(
            path.to_owned(),
            Document {
                path: path.into(),
                blob: entry.oid.clone(),
                language: language.into(),
            },
        );
        for occ in optional_array(doc, "occurrences")? {
            count += 1;
            if count > MAX_OCCURRENCES {
                return refuse("SCIP occurrence limit exceeded");
            }
            let symbol = opt(occ, "symbol")
                .map(Value::str)
                .transpose()?
                .unwrap_or("");
            if symbol.is_empty() {
                continue; // syntax highlighting without symbol reference
            }
            if symbol.len() > 4096 {
                return refuse("SCIP symbol identity exceeds 4096 bytes");
            }
            let line = range_line(occ, &lines, charset)?;
            let role = match property(occ, "symbol_roles", "symbolRoles") {
                Some(v) => num(v)?,
                None => 0,
            };
            // SCIP local symbols are document-scoped and cannot be joined by
            // their relative string IDs across documents.
            if symbol.starts_with("local ") || role & 16 != 0 {
                continue;
            }
            if role & 1 != 0 {
                definitions
                    .entry(symbol.into())
                    .or_default()
                    .insert(path.into());
            } else if role & 64 == 0 {
                references.push((path.to_owned(), symbol.to_owned(), line));
            }
        }
    }
    reader.finish()?;

    // Only a *unique* definition in the supplied index supports a local
    // reference. Ambiguous and external identities remain unsupported.
    let mut matched = BTreeSet::new();
    let mut unresolved = 0_usize;
    for (from, symbol, line) in &references {
        match definitions.get(symbol) {
            Some(destinations) if destinations.len() == 1 => {
                let to = destinations.iter().next().expect("unique definition");
                if to != from {
                    matched.insert(Reference {
                        from: from.clone(),
                        to: to.clone(),
                        symbol: symbol.clone(),
                        line: *line,
                    });
                }
            }
            _ => unresolved += 1,
        }
    }

    let mut by_pair: BTreeMap<(String, String), Vec<Reference>> = BTreeMap::new();
    for edge in &matched {
        by_pair
            .entry((edge.from.clone(), edge.to.clone()))
            .or_default()
            .push(edge.clone());
    }

    let mut candidates = Vec::new();
    let mut structural_candidate_ids = BTreeSet::new();
    let mut review_candidate_ids = BTreeSet::new();
    // A reciprocal *symbol reference* is not necessarily a compiler import
    // cycle; keep this distinct from the legacy dependency_cycle hypothesis.
    for ((left, right), forward) in &by_pair {
        if left >= right {
            continue;
        }
        if let Some(backward) = by_pair.get(&(right.clone(), left.clone())) {
            let evidence = forward
                .iter()
                .chain(backward.iter())
                .take(MAX_REFERENCES_PER_CANDIDATE)
                .map(|x| reference(x, &documents))
                .collect::<Vec<_>>();
            let likely_structure = likely_rust_module_structure(
                name,
                &documents[left],
                &documents[right],
                forward,
                backward,
            );
            let item = candidate(obj(&[
                ("kind", Value::string("reciprocal_symbol_references")),
                ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESIS")),
                ("sources", Value::array([source(&documents[left]), source(&documents[right])])),
                ("references", Value::array(evidence)),
                ("action", Value::string("Review reciprocal symbol use for a possible boundary or responsibility split; this is not proven to be an import cycle.")),
                ("verification", Value::string("Check language-specific compilation, generated code, runtime linkage and tests before modifying either file.")),
            ]))?;
            let id = item.get("candidate_id")?.str()?.to_owned();
            if likely_structure {
                structural_candidate_ids.insert(id);
            } else {
                review_candidate_ids.insert(id);
            }
            candidates.push(item);
        }
    }

    for edge in &matched {
        let from = &documents[&edge.from];
        let to = &documents[&edge.to];
        if from.language != "unspecified"
            && to.language != "unspecified"
            && from.language != to.language
        {
            let item = candidate(obj(&[
                ("kind", Value::string("cross_language_symbol_boundary")),
                ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESIS")),
                ("reference", reference(edge, &documents)),
                ("action", Value::string("Inspect the explicitly indexed cross-language symbol boundary and its ownership; do not infer FFI or API equivalence.")),
                ("verification", Value::string("Independently verify the language-specific bridge, build configurations and integration tests.")),
            ]))?;
            review_candidate_ids.insert(item.get("candidate_id")?.str()?.to_owned());
            candidates.push(item);
        }
    }
    if candidates.len() > MAX_CANDIDATES {
        return refuse("SCIP refactor candidate limit exceeded, narrow the index");
    }
    // Present review candidates first, but neither suppress nor alter the
    // original content-addressed candidate identities or their references.
    candidates.sort_by_key(|item| {
        let id = item.get("candidate_id").and_then(Value::str).unwrap_or("");
        (structural_candidate_ids.contains(id), item.canonical())
    });
    let mut report = obj(&[
        ("schema", Value::string("excession.scip-refactor.v1")),
        ("status", Value::string("UNVERIFIED_SCIP_RELATIONS")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("index_sha256", Value::string(crate::digest::sha256(bytes))),
        (
            "indexer",
            obj(&[
                ("name", Value::string(name)),
                ("version", Value::string(version)),
            ]),
        ),
        (
            "coverage",
            obj(&[
                ("indexed_documents", Value::number(documents.len())),
                ("indexed_occurrences", Value::number(count)),
                (
                    "unique_defined_symbols",
                    Value::number(definitions.values().filter(|v| v.len() == 1).count()),
                ),
                ("unresolved_references", Value::number(unresolved)),
                (
                    "matched_cross_file_references",
                    Value::number(matched.len()),
                ),
                ("line_only_position_documents", Value::number(lines_only)),
                (
                    "languages",
                    Value::array(languages.into_iter().map(Value::string)),
                ),
                (
                    "limitations",
                    Value::string(
                        "A SCIP indexer report is not proof of correctness or build completeness. References are not necessarily calls or imports. Absence of a reference is not evidence of absence. Cross-language identities are joined only by exact SCIP symbol IDs.",
                    ),
                ),
            ]),
        ),
        ("candidates", Value::array(candidates)),
        ("triage", obj(&[
            ("basis", Value::string("ADVISORY_RUST_ANALYZER_TOP_LEVEL_MODULE_SYMBOL")),
            ("review_candidate_ids", Value::array(review_candidate_ids.into_iter().map(Value::string))),
            ("likely_structural_module_candidate_ids", Value::array(structural_candidate_ids.into_iter().map(Value::string))),
        ])),
    ]);
    if include_references {
        // Preserve the exact native SCIP reference predicate. Neither this
        // collection nor the indexer proves all possible references are known.
        report.entries_mut()?.insert(
            "schema".into(),
            Value::string("excession.scip-observations.v1"),
        );
        report.entries_mut()?.insert(
            "observations".into(),
            Value::array(
                matched
                    .iter()
                    .map(|item| reference(item, &documents))
                    .collect::<Vec<_>>(),
            ),
        );
    }
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    if report.canonical().len() > MAX_INDEX_BYTES {
        return refuse("SCIP refactor report exceeds 16 MiB");
    }
    Ok(report)
}

pub fn audit(repo: &Path, revision: &str, index: &Path, report: &Path) -> Result<Value> {
    audit_with_prefix(repo, revision, index, report, None)
}

pub fn audit_with_prefix(
    repo: &Path,
    revision: &str,
    index: &Path,
    report: &Path,
    source_prefix: Option<&str>,
) -> Result<Value> {
    let data = std::fs::read(report).map_err(|e| format!("SCIP report read: {e}"))?;
    if data.len() > MAX_INDEX_BYTES {
        return refuse("SCIP report exceeds 16 MiB");
    }
    let saved = json::parse(&data)?;
    let result = suggest_with_prefix(repo, revision, index, source_prefix)?;
    if saved != result {
        return refuse("SCIP refactor report differs from regenerated Git-bound observations");
    }
    Ok(obj(&[
        ("report_id", result.get("report_id")?.clone()),
        ("source_commit", result.get("source_commit")?.clone()),
        ("status", Value::string("REPLAYED_SCIP_INDEX")),
        ("indexer_independently_executed", Value::Bool(false)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_typed_ranges_use_zero_based_lines() {
        let lines = vec![10, 10];
        let a = obj(&[(
            "range",
            Value::array([Value::number(0), Value::number(1), Value::number(3)]),
        )]);
        assert_eq!(range_line(&a, &lines, 1).unwrap(), 0);
        let b = obj(&[(
            "TypedRange",
            obj(&[(
                "SingleLineRange",
                obj(&[
                    ("line", Value::number(1)),
                    ("start_character", Value::number(2)),
                    ("end_character", Value::number(4)),
                ]),
            )]),
        )]);
        assert_eq!(range_line(&b, &lines, 1).unwrap(), 1);
        let bad = obj(&[(
            "range",
            Value::array([Value::number(0), Value::number(0), Value::number(100)]),
        )]);
        assert!(range_line(&bad, &lines, 1).is_err());
    }
 
    #[test]
    fn crate_root_module_triangulation_is_conservative() {
        let root = Document {
            path: "core/src/lib.rs".into(),
            language: "rust".into(),
            blob: "a".repeat(40),
        };
        let module = Document {
            path: "core/src/semantic.rs".into(),
            language: "rust".into(),
            blob: "b".repeat(40),
        };
        let declaration = Reference {
            from: root.path.clone(),
            to: module.path.clone(),
            symbol: "rust-analyzer cargo excession 0.2.0 semantic/".into(),
            line: 1,
        };
        let shared_type = Reference {
            from: module.path.clone(),
            to: root.path.clone(),
            symbol: "rust-analyzer cargo excession 0.2.0 Result#".into(),
            line: 5,
        };
        assert!(likely_rust_module_structure(
            "rust-analyzer",
            &root,
            &module,
            &[declaration.clone()],
            &[shared_type.clone()]
        ));
        // A real API use from the crate root is not just module layout.
        let mut api_use = declaration.clone();
        api_use.symbol = "rust-analyzer cargo excession 0.2.0 semantic/prepare_sources().".into();
        assert!(!likely_rust_module_structure(
            "rust-analyzer",
            &root,
            &module,
            &[declaration.clone(), api_use],
            &[shared_type.clone()]
        ));
        assert!(!likely_rust_module_structure(
            "rust-analyzer",
            &root,
            &module,
            &[shared_type.clone()],
            &[declaration.clone()]
        ));
        assert!(!likely_rust_module_structure(
            "other-indexer",
            &root,
            &module,
            &[declaration.clone()],
            &[shared_type.clone()]
        ));
        let mut nested = module.clone();
        nested.path = "core/src/semantic/worker.rs".into();
        assert!(!likely_rust_module_structure(
            "rust-analyzer",
            &root,
            &nested,
            &[declaration],
            &[shared_type]
        ));
    }

    #[test]
    fn candidate_id_is_source_payload_identity_not_triage() {
        let payload = obj(&[
            ("kind", Value::string("reciprocal_symbol_references")),
            ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESIS")),
        ]);
        let expected = payload.sha256();
        let item = candidate(payload).unwrap();
        assert_eq!(item.get("candidate_id").unwrap().str().unwrap(), expected);
        assert!(item.get("triage").is_err());
    }

}
