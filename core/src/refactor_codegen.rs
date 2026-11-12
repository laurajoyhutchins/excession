//! Bounded, deterministic Rust item extraction driven by audited SCIP evidence.
//! ast-grep supplies top-level syntax ranges. It does not resolve Rust names;
//! generated edits are unverified until compiler, API, and test receipts exist.
use crate::json::{self, Value, obj};
use crate::{Result, digest, git, refuse, scip_refactor};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_OUTLINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 8;

struct Item {
    name: String,
    start: usize,
    end: usize,
    body: String,
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && name.len() <= 100
        && !matches!(
            name,
            "self" | "super" | "crate" | "mod" | "fn" | "use" | "pub"
        )
}

fn selection(names: &str) -> Result<BTreeSet<String>> {
    let selected = names.split(',').map(str::trim).collect::<Vec<_>>();
    if selected.is_empty()
        || selected.len() > MAX_ITEMS
        || selected.iter().any(|name| !identifier(name))
    {
        return refuse("extract-shared-module requires 1..8 plain Rust function names");
    }
    let unique = selected
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    if unique.len() != selected.len() {
        return refuse("duplicate extracted function name");
    }
    Ok(unique)
}

fn read_blob(repo: &Path, oid: &str) -> Result<String> {
    let mut reader = git::BlobReader::open(repo)?;
    let bytes = reader.read_capped(oid, MAX_SOURCE_BYTES)?;
    reader.finish()?;
    String::from_utf8(bytes).map_err(|_| "Rust source blob is not UTF-8".to_owned())
}

/// Use ast-grep's outline, not an Excession implementation of Rust syntax.
/// Stdin contains *Git blob bytes*, never an unverified working-tree file.
fn outline(ast_grep: &Path, source: &str) -> Result<Value> {
    let mut child = Command::new(ast_grep)
        .args([
            "outline",
            "--stdin",
            "--lang",
            "Rust",
            "--items",
            "all",
            "--json=compact",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ast-grep launch failed: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("ast-grep stdin unavailable")?
        .write_all(source.as_bytes())
        .map_err(|e| format!("ast-grep stdin failed: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("ast-grep execution failed: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "ast-grep outline refused: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if output.stdout.len() > MAX_OUTLINE_BYTES {
        return refuse("ast-grep outline exceeds bounded output limit");
    }
    json::parse(&output.stdout)
}

fn extracted_items(source: &str, outlines: &Value, names: &BTreeSet<String>) -> Result<Vec<Item>> {
    let files = outlines.array_ref()?;
    if files.len() != 1 {
        return refuse("ast-grep must report exactly one stdin Rust source");
    }
    let mut extracted = Vec::new();
    let items = files[0].get("items")?.array_ref()?;
    if items.len() > 10_000 {
        return refuse("ast-grep source has too many top-level items");
    }
    for match_item in items {
        let name = match_item.get("name")?.str()?;
        if !names.contains(name) {
            continue;
        }
        if match_item.get("symbolType")?.str()? != "function"
            || match_item.get("astKind")?.str()? != "function_item"
        {
            return refuse(format!(
                "selected item {name} is not a top-level Rust function"
            ));
        }
        let range = match_item.get("range")?.get("byteOffset")?;
        let start = usize::try_from(range.get("start")?.integer()?)
            .map_err(|_| "negative ast-grep source offset")?;
        let end = usize::try_from(range.get("end")?.integer()?)
            .map_err(|_| "negative ast-grep source offset")?;
        if start >= end || end > source.len() {
            return refuse("ast-grep range outside pinned Git source");
        }
        let body = source
            .get(start..end)
            .ok_or("ast-grep range not on Rust UTF-8 character boundary")?;
        if !body.starts_with(&format!("pub fn {name}(")) {
            return refuse(format!(
                "unsupported extraction signature for {name}: require pub fn"
            ));
        }
        // This first recipe does not relocate attributes, doc comments, or
        // qualified module-relative references. Do not silently leave docs behind.
        // Keep adjacent line/documentation comments with the declaration.
        // Attributes and block comments require a different AST-aware recipe.
        let mut item_start = source[..start].rfind('\n').map_or(0, |pos| pos + 1);
        if !source[item_start..start].trim().is_empty() {
            return refuse("ast-grep function does not start on its own source line");
        }
        while item_start > 0 {
            let previous_end = item_start - 1;
            let previous_start = source[..previous_end]
                .rfind('\n')
                .map_or(0, |pos| pos + 1);
            let previous = source[previous_start..previous_end].trim();
            if previous.starts_with("//") && !previous.starts_with("//!") {
                item_start = previous_start;
            } else if previous.starts_with("#[")
                || previous.starts_with("/*")
                || previous.ends_with("*/")
                || previous.starts_with("//!")
            {
                return refuse(format!(
                    "unsupported attributes or module/block documentation on {name}"
                ));
            } else {
                break;
            }
        }
        if body.contains("super::") || body.contains("self::") || body.contains("crate::") {
            return refuse(format!("module-relative reference in extracted function {name}"));
        }
        extracted.push(Item {
            name: name.to_owned(),
            start: item_start,
            end,
            body: source[item_start..end].to_owned(),
        });
    }
    extracted.sort_by_key(|item| item.start);
    if extracted.len() != names.len()
        || extracted.windows(2).any(|pair| pair[0].end > pair[1].start)
        || extracted.iter().any(|item| item.end > source.len())
    {
        return refuse("missing, duplicated, or overlapping ast-grep function items");
    }
    Ok(extracted)
}

fn edits(
    source: &str,
    root: &str,
    module: &str,
    extracted: &[Item],
) -> Result<(String, String, String)> {
    if extracted.is_empty() || !identifier(module) {
        return refuse("unsupported destination module name");
    }
    let declaration = format!("mod {module};");
    if root.lines().any(|line| line.trim() == declaration)
        || root
            .lines()
            .any(|line| line.trim() == format!("pub {declaration}"))
    {
        return refuse("destination module is already declared");
    }
    let mut reduced = source.to_owned();
    for item in extracted.iter().rev() {
        reduced.replace_range(item.start..item.end, "");
    }
    // The replacement is placed at the first extracted item's original
    // position. Other top-level source references keep their original paths.
    let names = extracted
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    let export = format!("pub use crate::{module}::{{{}}};", names.join(", "));
    reduced.insert_str(extracted[0].start, &export);
    let mut destination = extracted
        .iter()
        .map(|item| item.body.trim_end())
        .collect::<Vec<_>>()
        .join("\n\n");
    destination.push('\n');
    // Preserve all original root bytes, including trailing whitespace.
    let separator = if root.ends_with('\n') { "" } else { "\n" };
    let updated_root = format!("{root}{separator}{declaration}\n");
    Ok((reduced, destination, updated_root))
}

fn assert_candidate(
    report: &Value,
    id: &str,
    source: &str,
    blob: &str,
    selected: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    if report.get("indexer")?.get("name")?.str()? != "rust-analyzer" {
        return refuse("recipe only supports rust-analyzer SCIP evidence");
    }
    let candidates = report.get("candidates")?.array_ref()?;
    let matches = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .get("candidate_id")
                .ok()
                .and_then(|v| v.str().ok())
                == Some(id)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return refuse("candidate ID missing or ambiguous in audited SCIP report");
    }
    let found = matches[0];
    if found.get("kind")?.str()? != "reciprocal_symbol_references" {
        return refuse("recipe requires reciprocal source-bound symbol references");
    }
    if !found.get("sources")?.array_ref()?.iter().any(|v| {
        v.get("path").ok().and_then(|x| x.str().ok()) == Some(source)
            && v.get("blob").ok().and_then(|x| x.str().ok()) == Some(blob)
            && v.get("language").ok().and_then(|x| x.str().ok()) == Some("rust")
    }) {
        return refuse("chosen source file and blob are not bound to the SCIP candidate");
    }
    if report
        .get("triage")?
        .get("likely_structural_module_candidate_ids")?
        .array_ref()?
        .iter()
        .any(|v| v.str().ok() == Some(id))
    {
        return refuse("ordinary structural module relationship is not an extraction target");
    }
    // A file participating in a reciprocal reference is not evidence that
    // *any* arbitrary function in that file is a useful extraction. Require
    // at least one selected public function to be the native symbol targeted
    // by the candidate. Other selected functions are explicit helper moves.
    let mut linked = BTreeSet::new();
    for reference in found.get("references")?.array_ref()? {
        let destination = reference.get("to")?;
        if destination.get("path")?.str()? != source
            || destination.get("blob")?.str()? != blob
        {
            continue;
        }
        let symbol = reference.get("symbol")?.str()?;
        for name in selected {
            if symbol.ends_with(&format!("/{name}().")) {
                linked.insert(name.clone());
            }
        }
    }
    if linked.is_empty() {
        return refuse("selected functions are unrelated to the cited SCIP references");
    }
    Ok(linked)
}

/// One edit over the original Git blob; ranges never depend on prior edits.
fn edit_operation(
    path: &str,
    before: Option<&str>,
    start: usize,
    end: usize,
    replacement: &str,
) -> Value {
    obj(&[
        ("path", Value::string(path)),
        ("original_blob", before.map(Value::string).unwrap_or(Value::Null)),
        ("start_byte", Value::number(start)),
        ("end_byte", Value::number(end)),
        ("replacement", Value::string(replacement)),
    ])
}

/// Independent, bounded replay of the byte ranges that will be written.
fn replay_edits(original: &str, operations: &Value, path: &str) -> Result<String> {
    let mut edits = Vec::new();
    for edit in operations.array_ref()? {
        if edit.get("path")?.str()? != path {
            continue;
        }
        let start = usize::try_from(edit.get("start_byte")?.integer()?)
            .map_err(|_| "negative generated edit offset")?;
        let end = usize::try_from(edit.get("end_byte")?.integer()?)
            .map_err(|_| "negative generated edit offset")?;
        if start > end || end > original.len()
            || !original.is_char_boundary(start) || !original.is_char_boundary(end)
        {
            return refuse("generated edit crosses a source boundary");
        }
        edits.push((start, end, edit.get("replacement")?.str()?));
    }
    edits.sort_by_key(|edit| (edit.0, edit.1));
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return refuse("generated edits overlap");
    }
    let mut result = original.to_owned();
    for (start, end, replacement) in edits.into_iter().rev() {
        result.replace_range(start..end, replacement);
    }
    Ok(result)
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub index: &'a Path,
    pub report: &'a Path,
    pub candidate: &'a str,
    pub source: &'a str,
    pub destination: &'a str,
    pub symbols: &'a str,
    pub ast_grep: &'a Path,
    pub source_prefix: Option<&'a str>,
    pub output: Option<&'a Path>,
}

pub fn extract_shared_module(request: &Request<'_>) -> Result<Value> {
    let repo = request.repo;
    let revision = git::rev(repo, request.revision)?;
    git::validate_path(request.source)?;
    git::validate_path(request.destination)?;
    let names = selection(request.symbols)?;
    let (directory, filename) = request
        .source
        .rsplit_once('/')
        .ok_or("extraction source must be below src/")?;
    if !(directory == "src" || directory.ends_with("/src"))
        || matches!(filename, "lib.rs" | "main.rs" | "mod.rs")
        || !filename.ends_with(".rs")
    {
        return refuse("recipe only supports a non-root src/*.rs module");
    }
    let module = request
        .destination
        .strip_prefix(directory)
        .and_then(|tail| tail.strip_prefix('/'))
        .and_then(|tail| tail.strip_suffix(".rs"))
        .ok_or("destination must be a sibling src/<new_module>.rs file")?;
    if !identifier(module) || module.contains('/') || module == "mod" {
        return refuse("unsupported destination module");
    }
    let entries = git::entries(repo, &revision)?;
    if entries.iter().any(|e| e.path == request.destination) {
        return refuse("destination is already in the pinned Git tree");
    }
    let root_path = format!("{directory}/lib.rs");
    let source_entry = git::entry_at(repo, &revision, request.source)?;
    let root_entry = git::entry_at(repo, &revision, &root_path)?;
    if source_entry.mode != "100644" || root_entry.mode != "100644" {
        return refuse("recipe only supports regular, non-executable Rust source blobs");
    }
    // Recompute and compare all candidate evidence from the exact index bytes.
    scip_refactor::audit_with_prefix(
        repo,
        &revision,
        request.index,
        request.report,
        request.source_prefix,
    )?;
    let report_bytes =
        fs::read(request.report).map_err(|e| format!("SCIP candidate report read: {e}"))?;
    let report = json::parse(&report_bytes)?;
    let linked = assert_candidate(
        &report,
        request.candidate,
        request.source,
        &source_entry.oid,
        &names,
    )?;
    let source = read_blob(repo, &source_entry.oid)?;
    let root = read_blob(repo, &root_entry.oid)?;
    let syntax = outline(request.ast_grep, &source)?;
    let extracted = extracted_items(&source, &syntax, &names)?;
    let (updated_source, created_module, updated_root) = edits(&source, &root, module, &extracted)?;
    // Exact byte edits against the original blobs make preview actionable,
    // independently replayable, and stable across preview/apply invocations.
    let export = format!(
        "pub use crate::{module}::{{{}}};",
        extracted.iter().map(|item| item.name.as_str()).collect::<Vec<_>>().join(", ")
    );
    let mut operations = Vec::new();
    for (position, item) in extracted.iter().enumerate() {
        operations.push(edit_operation(
            request.source,
            Some(&source_entry.oid),
            item.start,
            item.end,
            if position == 0 { &export } else { "" },
        ));
    }
    let suffix = if root.ends_with('\n') { "" } else { "\n" };
    operations.push(edit_operation(
        &root_path,
        Some(&root_entry.oid),
        root.len(),
        root.len(),
        &format!("{suffix}mod {module};\n"),
    ));
    operations.push(edit_operation(
        request.destination,
        None,
        0,
        0,
        &created_module,
    ));
    let operation_values = Value::array(operations);
    // The edits must reproduce every generated byte before a worktree is made.
    if replay_edits(&source, &operation_values, request.source)? != updated_source
        || replay_edits(&root, &operation_values, &root_path)? != updated_root
        || replay_edits("", &operation_values, request.destination)? != created_module
    {
        return refuse("generated extraction plan does not replay to its outputs");
    }
    let plan = obj(&[
        ("source_commit", Value::string(&revision)),
        ("candidate_id", Value::string(request.candidate)),
        ("scip_report_id", report.get("report_id")?.clone()),
        ("edits", operation_values.clone()),
    ]);
    let plan_id = plan.sha256();

    let mut status = "UNVERIFIED_CODEGEN_PREVIEW";
    let mut workspace = None;
    if let Some(output) = request.output {
        if !output.is_absolute() || output.exists() {
            return refuse("output worktree must be a new absolute path");
        }
        let parent = output
            .parent()
            .ok_or("worktree output has no parent")?
            .canonicalize()
            .map_err(|e| format!("worktree parent: {e}"))?;
        let repository = repo.canonicalize()
            .map_err(|e| format!("source checkout: {e}"))?;
        if parent.starts_with(&repository) {
            return refuse("generated worktree cannot be inside the original checkout");
        }
        let output_str = output.to_str().ok_or("non-UTF-8 worktree output path")?;
        git::command(
            repo,
            &["-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", output_str, &revision],
        )?;
        let write_result = (|| -> Result<()> {
            fs::write(output.join(request.source), &updated_source)
                .map_err(|e| format!("write extracted source: {e}"))?;
            fs::write(output.join(request.destination), &created_module)
                .map_err(|e| format!("write new module: {e}"))?;
            fs::write(output.join(&root_path), &updated_root)
                .map_err(|e| format!("write crate root: {e}"))?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = git::command(
                repo,
                &["-c", "core.hooksPath=/dev/null", "worktree", "remove", "--force", output_str],
            );
            return Err(error);
        }
        status = "APPLIED_UNVERIFIED";
        workspace = Some(output_str.to_owned());
    }
    let changes = [
        (request.source, source_entry.oid.as_str(), &updated_source),
        (request.destination, "", &created_module),
        (root_path.as_str(), root_entry.oid.as_str(), &updated_root),
    ];
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.refactor-extract-shared-module.v1"),
        ),
        ("status", Value::string(status)),
        ("source_commit", Value::string(revision)),
        ("candidate_id", Value::string(request.candidate)),
        ("scip_report_id", report.get("report_id")?.clone()),
        ("recipe", Value::string("extract-shared-module")),
        ("plan_id", Value::string(plan_id)),
        ("edits", operation_values),
        ("linked_functions", Value::array(linked.iter().map(Value::string))),
        (
            "additional_helper_functions",
            Value::array(names.difference(&linked).map(Value::string)),
        ),
        (
            "worktree",
            workspace.map(Value::string).unwrap_or(Value::Null),
        ),
        (
            "functions",
            Value::array(extracted.iter().map(|item| Value::string(&item.name))),
        ),
        (
            "changes",
            Value::array(changes.into_iter().map(|(path, before, after)| {
                obj(&[
                    ("path", Value::string(path)),
                    (
                        "original_blob",
                        if before.is_empty() {
                            Value::Null
                        } else {
                            Value::string(before)
                        },
                    ),
                    (
                        "generated_sha256",
                        Value::string(digest::sha256(after.as_bytes())),
                    ),
                ])
            })),
        ),
        (
            "verification",
            Value::string(
                "UNRESOLVED: run rustfmt, compiler, API compatibility, full tests and fresh SCIP candidate replay on the isolated revision. No behavioral equivalence has been established.",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_outline(items: &[(&str, &str, usize, usize)]) -> Value {
        Value::array([obj(&[(
            "items",
            Value::array(
                items
                    .iter()
                    .map(|(name, kind, start, end)| {
                        obj(&[
                            ("name", Value::string(*name)),
                            ("symbolType", Value::string("function")),
                            ("astKind", Value::string(*kind)),
                            (
                                "range",
                                obj(&[(
                                    "byteOffset",
                                    obj(&[
                                        ("start", Value::number(*start)),
                                        ("end", Value::number(*end)),
                                    ]),
                                )]),
                            ),
                        ])
                    })
                    .collect::<Vec<_>>(),
            ),
        )])])
    }

    #[test]
    fn extracts_two_public_functions_and_keeps_compatibility_aliases() {
        let source = "pub fn one() -> i32 { 1 }\n\npub fn two() -> i32 { one() + 1 }\n";
        let a = "pub fn one() -> i32 { 1 }";
        let b = "pub fn two() -> i32 { one() + 1 }";
        let start_b = source.find(b).unwrap();
        let ast = fake_outline(&[
            ("one", "function_item", 0, a.len()),
            ("two", "function_item", start_b, start_b + b.len()),
        ]);
        let names = selection("two,one").unwrap();
        let found = extracted_items(source, &ast, &names).unwrap();
        let (reduced, moved, root) = edits(source, "pub mod module;", "shared", &found).unwrap();
        assert!(reduced.contains("pub use crate::shared::{one, two};"));
        assert!(!reduced.contains("pub fn one"));
        assert!(moved.contains(a) && moved.contains(b));
        assert!(root.contains("mod shared;"));
    }

    #[test]
    fn refuses_missing_ambiguous_unsafe_or_out_of_bounds_ranges() {
        let source = "pub fn foo() {}\n";
        let names = selection("foo").unwrap();
        let wrong = fake_outline(&[("foo", "struct_item", 0, 15)]);
        assert!(extracted_items(source, &wrong, &names).is_err());
        let missing = fake_outline(&[]);
        assert!(extracted_items(source, &missing, &names).is_err());
        let oversized = fake_outline(&[("foo", "function_item", 0, 1000)]);
        assert!(extracted_items(source, &oversized, &names).is_err());
        let duplicate = fake_outline(&[
            ("foo", "function_item", 0, source.trim_end().len()),
            ("foo", "function_item", 0, source.trim_end().len()),
        ]);
        assert!(extracted_items(source, &duplicate, &names).is_err());
        assert!(selection("foo,foo").is_err());
        assert!(selection("../foo").is_err());
        assert!(
            extracted_items(
                "/// documented\npub fn foo() {}\n",
                &fake_outline(&[("foo", "function_item", 15, 30)]),
                &names,
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_existing_destination_declaration() {
        let source = "pub fn foo() {}";
        let items = [Item {
            name: "foo".into(),
            start: 0,
            end: source.len(),
            body: source.into(),
        }];
        assert!(edits(source, "mod shared;\n", "shared", &items).is_err());
    }
}
