//! Strict, source-identity-preserving completion for one native Extract Module action.
//! This is a byte-conservative bridge, not a general Rust rewriter. We only
//! admit a *single unchanged public function* wrapped by rust-analyzer in a
//! new private inline module. Everything else must use a separate recipe.
use crate::json::{Value, obj};
use crate::{Result, git, refactor_codegen, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_HELPERS: usize = 4;
const MAX_ITEMS: usize = 8;
const MAX_FILE: usize = 512 * 1024;

pub(crate) struct Missing<'a> {
    pub path: &'a str,
    pub line: usize,
    pub column: usize,
    pub code: &'a str,
    pub message: &'a str,
}

pub(crate) struct Proposal {
    pub path: String,
    pub edits: Value,
    pub outputs: BTreeMap<String, String>,
    pub public_functions: Vec<String>,
    pub helpers: Vec<String>,
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && value.len() <= 100
        && !matches!(
            value,
            "self" | "super" | "crate" | "mod" | "fn" | "pub" | "use"
        )
}

/// Require a byte-identical native module wrapper. The upstream LSP supplies
/// the edit; AST item identities are independently checked against original Git
/// bytes below, using the existing ast-grep outline dependency.
///
/// Empty separator lines may be blank or four-space-indented. Nonblank lines
/// must have exactly the provider-added four-space prefix.
fn unchanged_move(original: &str, replacement: &str) -> Result<Option<(String, usize)>> {
    let Some((header, body)) = replacement.split_once('\n') else {
        return Ok(None);
    };
    let Some(name) = header
        .strip_prefix("mod ")
        .and_then(|h| h.strip_suffix(" {"))
    else {
        return Ok(None);
    };
    if !identifier(name) {
        return Ok(None);
    }
    let Some(body) = body.strip_suffix("\n}") else {
        return Ok(None);
    };
    let mut deindented = Vec::new();
    for line in body.split('\n') {
        if line.is_empty() {
            deindented.push("");
        } else if let Some(line) = line.strip_prefix("    ") {
            deindented.push(line);
        } else {
            return refuse("native extraction changed indentation or moved unsupported items");
        }
    }
    if deindented.join("\n") != original {
        return refuse("native extraction changed original Rust source bytes");
    }
    if original
        .lines()
        .any(|line| line.trim_start().starts_with("#["))
        || original.contains("super::")
        || original.contains("self::")
        || original.contains("crate::")
    {
        return Ok(None);
    }
    Ok(Some((name.to_owned(), header.len() + 1)))
}

fn declared(source: &str, name: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| {
        source
            .strip_prefix(prefix)
            .and_then(|tail| tail.strip_prefix(name))
            .is_some_and(|tail| tail.starts_with('(') || tail.starts_with('<'))
    })
}

fn safe_gap(gap: &str) -> bool {
    gap.lines().all(|line| {
        let line = line.trim();
        line.is_empty() || (line.starts_with("//") && !line.starts_with("//!"))
    })
}

struct MoveItems {
    public_functions: Vec<String>,
    parent_helpers: BTreeSet<String>,
}

/// Trust ast-grep's top-level function ranges rather than an Excession line
/// scanner. Refuse every non-function item intersecting the moved interval,
/// ambiguous/overlapping ranges, unsupported visibility, and unexplained gaps.
fn moved_items(
    source: &str,
    start: usize,
    end: usize,
    outline: &Value,
) -> Result<Option<MoveItems>> {
    let files = outline.array_ref()?;
    if files.len() != 1 {
        return refuse("ast-grep must return exactly one original Rust source file");
    }
    let items = files[0].get("items")?.array_ref()?;
    if items.len() > 10_000 {
        return refuse("ast-grep source item count exceeds bound");
    }
    let mut moved = Vec::new();
    let mut helpers = BTreeSet::new();
    for item in items {
        let name = item.get("name")?.str()?;
        let kind = item.get("astKind")?.str()?;
        let offsets = item.get("range")?.get("byteOffset")?;
        let begin = usize::try_from(offsets.get("start")?.integer()?)
            .map_err(|_| "negative ast-grep item start")?;
        let finish = usize::try_from(offsets.get("end")?.integer()?)
            .map_err(|_| "negative ast-grep item end")?;
        if begin >= finish || finish > source.len() {
            return refuse("ast-grep item range outside Git source");
        }
        let body = source
            .get(begin..finish)
            .ok_or("ast-grep splits UTF-8 source")?;
        let is_function = kind == "function_item"
            && item.get("symbolType")?.str()? == "function"
            && identifier(name);
        if begin < end && finish > start {
            if begin < start || finish > end || !is_function {
                return refuse("moved interval includes a partial or non-function syntax item");
            }
            if !declared(
                body,
                name,
                &[
                    "pub fn ",
                    "pub async fn ",
                    "pub const fn ",
                    "pub unsafe fn ",
                ],
            ) {
                return Ok(None);
            }
            moved.push((begin, finish, name.to_owned()));
        } else if is_function
            && declared(
                body,
                name,
                &[
                    "fn ",
                    "async fn ",
                    "const fn ",
                    "unsafe fn ",
                    "pub fn ",
                    "pub async fn ",
                    "pub(crate) fn ",
                    "pub(super) fn ",
                    "pub const fn ",
                ],
            )
        {
            helpers.insert(name.to_owned());
        }
    }
    moved.sort_by_key(|item| item.0);
    if moved.is_empty() || moved.len() > MAX_ITEMS {
        return Ok(None);
    }
    let mut cursor = start;
    let mut names = BTreeSet::new();
    let mut public_functions = Vec::new();
    for (begin, finish, name) in moved {
        if begin < cursor
            || !safe_gap(source.get(cursor..begin).ok_or("invalid moved gap")?)
            || !names.insert(name.clone())
        {
            return refuse("moved items overlap, duplicate names, or include unsupported syntax");
        }
        cursor = finish;
        public_functions.push(name);
    }
    if !safe_gap(
        source
            .get(cursor..end)
            .ok_or("invalid trailing moved gap")?,
    ) {
        return refuse("unsupported source between extracted items");
    }
    Ok(Some(MoveItems {
        public_functions,
        parent_helpers: helpers,
    }))
}

fn line_byte(text: &str, line: usize, column: usize) -> Result<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text[start..]
            .find('\n')
            .ok_or("compiler line outside source")?
            + 1;
    }
    let end = text[start..].find('\n').map_or(text.len(), |n| start + n);
    if !text[start..end].is_ascii() {
        return refuse("non-ASCII compiler line needs an explicit semantic action");
    }
    if column > end - start {
        return refuse("compiler column outside source line");
    }
    Ok(start + column)
}

fn missing_function(message: &str) -> Option<&str> {
    message
        .strip_prefix("cannot find function `")?
        .strip_suffix("` in this scope")
        .filter(|name| identifier(name))
}

/// Pure planning boundary, tested without a language server or Git worktree.
fn preserve_bytes(
    path: &str,
    oid: &str,
    old_source: &str,
    current_source: &str,
    prior_edits: &Value,
    outline: &Value,
    errors: &[Missing<'_>],
) -> Result<Option<Proposal>> {
    let edits = prior_edits.array_ref()?;
    if edits.len() != 1 {
        return Ok(None);
    }
    let change = &edits[0];
    if change.get("path")?.str()? != path {
        return Ok(None);
    }
    if change.get("original_blob")?.str()? != oid {
        return refuse("native extraction edit is not bound to original Git source blob");
    }
    let start = usize::try_from(change.get("start_byte")?.integer()?)
        .map_err(|_| "invalid native extraction start")?;
    let end = usize::try_from(change.get("end_byte")?.integer()?)
        .map_err(|_| "invalid native extraction end")?;
    if start >= end
        || end > old_source.len()
        || !old_source.is_char_boundary(start)
        || !old_source.is_char_boundary(end)
    {
        return refuse("native extracted range is not a valid UTF-8 source range");
    }
    let replacement = change.get("replacement")?.str()?;
    let Some((module, header_bytes)) = unchanged_move(&old_source[start..end], replacement)? else {
        return Ok(None);
    };
    let Some(items) = moved_items(old_source, start, end, outline)? else {
        return Ok(None);
    };
    let mut expected = old_source.to_owned();
    expected.replace_range(start..end, replacement);
    if expected != current_source {
        return refuse("native candidate is not exactly the original single-item source edit");
    }
    let module_begin = start + header_bytes;
    let module_end = start + replacement.len();
    let mut helpers = BTreeSet::new();
    for error in errors {
        if error.path != path {
            continue;
        }
        let pos = line_byte(current_source, error.line, error.column)?;
        if pos < module_begin || pos >= module_end {
            continue;
        }
        if error.code != "E0425" {
            return refuse(
                "native extraction has unsupported unresolved names in the moved module",
            );
        }
        let Some(helper) = missing_function(error.message) else {
            return refuse("unrecognized native extracted-module compiler diagnostic");
        };
        if items.public_functions.iter().any(|name| name == helper)
            || !current_source[pos..].starts_with(helper)
            || !current_source[pos + helper.len()..].starts_with('(')
            || !items.parent_helpers.contains(helper)
        {
            return refuse("compiler error is not a verified parent-module function dependency");
        }
        helpers.insert(helper.to_owned());
        if helpers.len() > MAX_HELPERS {
            return refuse("too many unresolved parent helpers in native extraction");
        }
    }
    // Two inserts over *one committed Git blob*. The native function body
    // remains unchanged; the old parent-level public path is restored.
    let imports = helpers
        .iter()
        .map(|h| format!("    use super::{h};\n"))
        .collect::<String>();
    let export = if items.public_functions.len() == 1 {
        format!("\npub use self::{module}::{};", items.public_functions[0])
    } else {
        format!(
            "\npub use self::{module}::{{{}}};",
            items.public_functions.join(", ")
        )
    };
    let changes = Value::array([
        refactor_codegen::edit_operation(path, Some(oid), module_begin, module_begin, &imports),
        refactor_codegen::edit_operation(path, Some(oid), module_end, module_end, &export),
    ]);
    let updated = refactor_codegen::replay_edits(current_source, &changes, path)?;
    if updated.len() > MAX_FILE {
        return refuse("preserved native Rust source exceeds maximum file size");
    }
    let mut outputs = BTreeMap::new();
    outputs.insert(path.to_owned(), updated);
    Ok(Some(Proposal {
        path: path.to_owned(),
        edits: changes,
        outputs,
        public_functions: items.public_functions,
        helpers: helpers.into_iter().collect(),
    }))
}

pub(crate) fn propose(
    repo: &Path,
    base: &str,
    parent: &Value,
    ast_grep: &Path,
    errors: &[Missing<'_>],
) -> Result<Option<Proposal>> {
    if parent.get("action_title")?.str()? != "Extract Module"
        || parent.get("action_kind")?.str()? != "refactor.extract"
    {
        return Ok(None);
    }
    let changes = parent.get("changes")?.array_ref()?;
    if changes.len() != 1 {
        return Ok(None);
    }
    let path = changes[0].get("path")?.str()?;
    git::validate_path(path)?;
    if !path.ends_with(".rs") {
        return Ok(None);
    }
    let original = git::entry_at(repo, base, path)?;
    if original.kind != "blob" || original.mode != "100644" {
        return refuse("native source must be a regular non-executable Git blob");
    }
    let mut reader = git::BlobReader::open(repo)?;
    let bytes = reader.read_capped(&original.oid, MAX_FILE)?;
    reader.finish()?;
    let original_source = String::from_utf8(bytes).map_err(|_| "old Rust source is not UTF-8")?;
    let current_entry = git::entry_at(repo, &git::rev(repo, "HEAD")?, path)?;
    if current_entry.kind != "blob" || current_entry.mode != "100644" {
        return refuse("native candidate source must remain a regular Git blob");
    }
    let mut reader = git::BlobReader::open(repo)?;
    let bytes = reader.read_capped(&current_entry.oid, MAX_FILE)?;
    reader.finish()?;
    let current_source =
        String::from_utf8(bytes).map_err(|_| "candidate Rust source is not UTF-8")?;
    let syntax = refactor_codegen::outline(ast_grep, &original_source)?;
    preserve_bytes(
        path,
        &original.oid,
        &original_source,
        &current_source,
        parent.get("edits")?,
        &syntax,
        errors,
    )
    .map(|proposal| {
        proposal.map(|mut result| {
            // Translate original-blob anchors to the *committed candidate*
            // revision, because that is the exact input to the second edit.
            let edits = result.edits.array_ref().expect("constructed local edits");
            result.edits = Value::array(edits.iter().map(|item| {
                obj(&[
                    ("path", item.get("path").expect("local path").clone()),
                    ("original_blob", Value::string(&current_entry.oid)),
                    (
                        "start_byte",
                        item.get("start_byte").expect("local range").clone(),
                    ),
                    (
                        "end_byte",
                        item.get("end_byte").expect("local range").clone(),
                    ),
                    (
                        "replacement",
                        item.get("replacement").expect("local text").clone(),
                    ),
                ])
            }));
            result
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(items: &[(&str, &str, usize, usize)]) -> Value {
        Value::array([obj(&[(
            "items",
            Value::array(items.iter().map(|(name, kind, start, end)| {
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
            })),
        )])])
    }

    fn span(source: &str, text: &str) -> (usize, usize) {
        let start = source.find(text).unwrap();
        (start, start + text.len())
    }

    fn fixture() -> (String, String, Value, Value) {
        let source = "fn helper() -> usize { 7 }\n\
                      fn offset() -> usize { 3 }\n\
                      \n\
                      /// documented α\n\
                      pub fn first<T: Copy>(x: T) -> T {\n    let _ = helper();\n    x\n}\n\
                      \n\
                      pub const fn second() -> usize { 9 }\n\
                      \n\
                      pub fn third() -> usize {\n    helper() + offset()\n}\n\
                      \n\
                      fn caller() -> usize { third() }\n";
        let first = "pub fn first<T: Copy>(x: T) -> T {\n    let _ = helper();\n    x\n}";
        let second = "pub const fn second() -> usize { 9 }";
        let third = "pub fn third() -> usize {\n    helper() + offset()\n}";
        let moved = format!("/// documented α\n{first}\n\n{second}\n\n{third}");
        let start = source.find(&moved).unwrap();
        let wrapper = format!(
            "mod extracted {{\n{}\n}}",
            moved
                .lines()
                .map(|line| if line.is_empty() {
                    String::new()
                } else {
                    format!("    {line}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut candidate = source.to_owned();
        candidate.replace_range(start..start + moved.len(), &wrapper);
        let edit = refactor_codegen::edit_operation(
            "src/example.rs",
            Some("old"),
            start,
            start + moved.len(),
            &wrapper,
        );
        let ast = outline(&[
            (
                "helper",
                "function_item",
                span(source, "fn helper() -> usize { 7 }").0,
                span(source, "fn helper() -> usize { 7 }").1,
            ),
            (
                "offset",
                "function_item",
                span(source, "fn offset() -> usize { 3 }").0,
                span(source, "fn offset() -> usize { 3 }").1,
            ),
            (
                "first",
                "function_item",
                span(source, first).0,
                span(source, first).1,
            ),
            (
                "second",
                "function_item",
                span(source, second).0,
                span(source, second).1,
            ),
            (
                "third",
                "function_item",
                span(source, third).0,
                span(source, third).1,
            ),
            (
                "caller",
                "function_item",
                span(source, "fn caller() -> usize { third() }").0,
                span(source, "fn caller() -> usize { third() }").1,
            ),
        ]);
        (source.to_owned(), candidate, Value::array([edit]), ast)
    }

    fn repair_fixture() -> (String, String) {
        let (old, candidate, edits, ast) = fixture();
        let line = candidate
            .lines()
            .position(|line| line.contains("        helper() + offset()"))
            .unwrap();
        let errors = [
            Missing {
                path: "src/example.rs",
                line,
                column: 8,
                code: "E0425",
                message: "cannot find function \x60helper\x60 in this scope",
            },
            Missing {
                path: "src/example.rs",
                line,
                column: 19,
                code: "E0425",
                message: "cannot find function \x60offset\x60 in this scope",
            },
        ];
        let plan = preserve_bytes(
            "src/example.rs",
            "old",
            &old,
            &candidate,
            &edits,
            &ast,
            &errors,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.public_functions, ["first", "second", "third"]);
        assert_eq!(plan.helpers, ["helper", "offset"]);
        let after = plan.outputs["src/example.rs"].clone();
        assert!(after.contains("pub use self::extracted::{first, second, third};"));
        assert!(after.contains("    use super::helper;\n    use super::offset;\n"));
        assert!(after.contains("/// documented α"));
        assert_eq!(
            refactor_codegen::replay_edits(&candidate, &plan.edits, "src/example.rs").unwrap(),
            after
        );
        (old, after)
    }

    #[test]
    fn multiple_public_generic_const_functions_and_helpers() {
        repair_fixture();
    }

    #[test]
    fn refuse_malformed_ast_bodies_and_unsupported_visibility() {
        let (before, candidate, edits, ast) = fixture();
        let corrupt = candidate.replace("        helper() + offset()", "        999");
        assert!(
            preserve_bytes(
                "src/example.rs",
                "old",
                &before,
                &corrupt,
                &edits,
                &ast,
                &[]
            )
            .is_err()
        );
        let bad = outline(&[("first", "function_item", 0, before.len() + 1)]);
        assert!(
            preserve_bytes(
                "src/example.rs",
                "old",
                &before,
                &candidate,
                &edits,
                &bad,
                &[]
            )
            .is_err()
        );
        let partial = outline(&[("first", "function_item", 0, before.len() / 2)]);
        assert!(
            preserve_bytes(
                "src/example.rs",
                "old",
                &before,
                &candidate,
                &edits,
                &partial,
                &[]
            )
            .is_err()
        );
        let wrong = outline(&[(
            "first",
            "struct_item",
            span(&before, "pub fn first").0,
            span(&before, "pub fn first").1,
        )]);
        assert!(
            preserve_bytes(
                "src/example.rs",
                "old",
                &before,
                &candidate,
                &edits,
                &wrong,
                &[]
            )
            .is_err()
        );

        let old = "pub(crate) fn private() {}";
        let ast = outline(&[("private", "function_item", 0, old.len())]);
        assert!(moved_items(old, 0, old.len(), &ast).unwrap().is_none());

        let old = "pub fn a() {}\nconst SKIPPED: u8 = 1;\npub fn b() {}";
        let ast = outline(&[
            ("a", "function_item", 0, "pub fn a() {}".len()),
            (
                "b",
                "function_item",
                old.find("pub fn b").unwrap(),
                old.len(),
            ),
        ]);
        assert!(moved_items(old, 0, old.len(), &ast).is_err());

        let marked = "#[cfg(feature=\"x\")]\npub fn item() {}";
        let wrapped = "mod extra {\n    #[cfg(feature=\"x\")]\n    pub fn item() {}\n}";
        assert!(unchanged_move(marked, wrapped).unwrap().is_none());
    }

    #[test]
    fn bounded_eight_item_selection_and_duplicate_guard() {
        let mut source = String::new();
        let mut entries = Vec::new();
        for n in 0..9 {
            let item = format!("pub fn f{n}() {{}}");
            let start = source.len();
            source.push_str(&item);
            entries.push((format!("f{n}"), start, source.len()));
            source.push('\n');
        }
        let ast = Value::array([obj(&[(
            "items",
            Value::array(entries.iter().map(|(name, start, end)| {
                obj(&[
                    ("name", Value::string(name)),
                    ("symbolType", Value::string("function")),
                    ("astKind", Value::string("function_item")),
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
            })),
        )])]);
        assert!(
            moved_items(&source, 0, source.len(), &ast)
                .unwrap()
                .is_none()
        );
        let duplicate = outline(&[("a", "function_item", 0, 12), ("a", "function_item", 0, 12)]);
        assert!(moved_items("pub fn a() {}", 0, 12, &duplicate).is_err());
    }

    #[test]
    fn rejects_unknown_parent_dependency_and_mismatched_diagnostic() {
        let (old, candidate, edits, ast) = fixture();
        let line = candidate
            .lines()
            .position(|line| line.contains("        helper() + offset()"))
            .unwrap();
        for (code, message) in [
            ("E0433", "unknown path"),
            (
                "E0425",
                "cannot find function \x60not_declared\x60 in this scope",
            ),
        ] {
            let errors = [Missing {
                path: "src/example.rs",
                line,
                column: 8,
                code,
                message,
            }];
            assert!(
                preserve_bytes(
                    "src/example.rs",
                    "old",
                    &old,
                    &candidate,
                    &edits,
                    &ast,
                    &errors
                )
                .is_err()
            );
        }
    }

    #[test]
    fn generated_multi_item_repair_compiles_and_matches_original_behavior() {
        let (old, after) = repair_fixture();
        let main = "\nfn main() { println!(\"{} {} {}\", first(5), second(), third()); }\n";
        let root = std::env::temp_dir().join(format!(
            "excession-api-stress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let run = |label: &str, body: &str| -> Vec<u8> {
            let source = root.join(format!("{label}.rs"));
            let binary = root.join(label);
            std::fs::write(&source, format!("{body}{main}")).unwrap();
            let compile = std::process::Command::new(&rustc)
                .args(["--crate-name", "stress", "--edition=2024"])
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compile.status.success(),
                "rustc: {}",
                String::from_utf8_lossy(&compile.stderr)
            );
            let output = std::process::Command::new(binary).output().unwrap();
            assert!(output.status.success());
            output.stdout
        };
        assert_eq!(run("original", &old), run("repaired", &after));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn accepts_exactly_eight_public_functions_including_async_and_unsafe() {
        let mut source=String::new();
        let mut specs=Vec::new();
        for number in 0..8 {
            let name=format!("function_{number}");
            let item=match number {
                0 => format!("pub async fn {name}<T: Copy>(value: T) -> T {{ value }}"),
                1 => format!("pub unsafe fn {name}() -> usize {{ 1 }}"),
                _ => format!("pub fn {name}() -> usize {{ {number} }}"),
            };
            let start=source.len();
            source.push_str(&item);
            specs.push((name,start,source.len()));
            source.push('\n');
        }
        let syntax=Value::array([obj(&[("items",Value::array(specs.iter().map(
            |(name,start,end)|obj(&[
                ("name",Value::string(name)),
                ("astKind",Value::string("function_item")),
                ("symbolType",Value::string("function")),
                ("range",obj(&[("byteOffset",obj(&[
                    ("start",Value::number(*start)),
                    ("end",Value::number(*end)),
                ]))])),
            ])
        )))])]);
        let accepted=moved_items(&source,0,source.len(),&syntax).unwrap().unwrap();
        assert_eq!(accepted.public_functions.len(),8);
        assert_eq!(accepted.public_functions[0],"function_0");
        assert_eq!(accepted.public_functions[7],"function_7");
    }

    #[test]
    #[ignore = "requires pinned ast-grep; exercised by native historical replay workflow"]
    fn external_pinned_corpus_stress() {
        let binary = std::env::var_os("EXCESSION_AST_GREP").expect("pinned ast-grep executable");
        let analyzer = std::path::Path::new(&binary);
        let corpus = [
            (
                "ripgrep",
                include_str!("../../tests/fixtures/rust-api/ripgrep-escape.rs"),
                "escape",
            ),
            (
                "serde_json",
                include_str!("../../tests/fixtures/rust-api/serde-json-from-str.rs"),
                "from_str",
            ),
        ];
        let mut generated = String::new();
        let mut original = String::new();
        for (project, source, function) in corpus {
            let syntax = refactor_codegen::outline(analyzer, source).unwrap();
            let marker = format!("pub fn {function}");
            let begin = source.find(&marker).unwrap();
            let end = source.trim_end().len();
            let items = moved_items(source, begin, end, &syntax).unwrap().unwrap();
            assert_eq!(items.public_functions, [function], "{project}");
            let moved = &source[begin..end];
            let wrapper = format!(
                "mod extracted {{\n{}\n}}",
                moved
                    .lines()
                    .map(|line| if line.is_empty() {
                        String::new()
                    } else {
                        format!("    {line}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let mut candidate = source.to_owned();
            candidate.replace_range(begin..end, &wrapper);
            let edits = Value::array([refactor_codegen::edit_operation(
                "src/external.rs",
                Some("old"),
                begin,
                end,
                &wrapper,
            )]);
            let patch = preserve_bytes(
                "src/external.rs",
                "old",
                source,
                &candidate,
                &edits,
                &syntax,
                &[],
            )
            .unwrap()
            .unwrap();
            let after = &patch.outputs["src/external.rs"];
            assert!(after.contains(&format!("pub use self::extracted::{function};")));
            assert_eq!(
                refactor_codegen::replay_edits(&candidate, &patch.edits, "src/external.rs")
                    .unwrap(),
                *after
            );
            if project == "ripgrep" {
                original = source.to_owned();
                generated = after.clone();
            }
        }
        // A genuine upstream function is dependency-free. Exercise the
        // generated result against rustc and compare output over several cases.
        let main = "\nfn main() { for glob in [\"a*b\",\"foo?\",\"[abc]\",\"plain\"] { println!(\"{}\", escape(glob)); } }\n";
        let tmp = std::env::temp_dir().join(format!(
            "excession-upstream-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let run = |name: &str, source: &str| {
            let file = tmp.join(format!("{name}.rs"));
            let executable = tmp.join(name);
            std::fs::write(&file, format!("{source}{main}")).unwrap();
            let compiled = std::process::Command::new(&rustc)
                .args(["--crate-name", "upstream", "--edition=2024"])
                .arg(file)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "rustc: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let finished = std::process::Command::new(executable).output().unwrap();
            assert!(finished.status.success());
            finished.stdout
        };
        assert_eq!(run("original", &original), run("repaired", &generated));
        std::fs::remove_dir_all(tmp).unwrap();

        // A public impl method is not a top-level public function: the old
        // prefix scan could have falsely accepted it. Require AST rejection.
        let nested = "struct Example;\nimpl Example {\n    pub fn method() {}\n}\n";
        let syntax = refactor_codegen::outline(analyzer, nested).unwrap();
        let begin = nested.find("pub fn method").unwrap();
        let end = begin + "pub fn method() {}".len();
        assert!(
            moved_items(nested, begin, end, &syntax).is_err()
                || moved_items(nested, begin, end, &syntax).unwrap().is_none()
        );
    }
}
