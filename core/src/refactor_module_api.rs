//! Strict, source-identity-preserving completion for one native Extract Module action.
//! This is a byte-conservative bridge, not a general Rust rewriter. We only
//! admit a *single unchanged public function* wrapped by rust-analyzer in a
//! new private inline module. Everything else must use a separate recipe.
use crate::json::{Value, obj};
use crate::{Result, git, refactor_codegen, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_HELPERS: usize = 4;
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
    pub moved_function: String,
    pub helpers: Vec<String>,
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && value.len() <= 100
        && !matches!(value, "self" | "super" | "crate" | "mod" | "fn" | "pub" | "use")
}

/// Restrict the recipe to this exactly verified rust-analyzer move:
///
///     pub fn f(...) { ... }  ->  mod name { pub fn f(...) { ... } }
///
/// Both the original declaration and its entire function body must be
/// byte-identical after removing the provider's four-space indentation.
fn unchanged_move(original: &str, replacement: &str) -> Result<Option<(String, String, usize)>> {
    let Some((header, body)) = replacement.split_once('\n') else {
        return Ok(None);
    };
    let Some(name) = header.strip_prefix("mod ").and_then(|h| h.strip_suffix(" {")) else {
        return Ok(None);
    };
    if !identifier(name) { return Ok(None); }
    let Some(body) = body.strip_suffix("\n}") else {
        return Ok(None);
    };
    let mut deindented = Vec::new();
    for line in body.split('\n') {
        let Some(line) = line.strip_prefix("    ") else {
            return refuse("native extraction changed indentation or moved unsupported items");
        };
        deindented.push(line);
    }
    if deindented.join("\n") != original {
        return refuse("native extraction changed Rust function source bytes");
    }
    let exported = original.lines().filter_map(|line| {
        line.strip_prefix("pub fn ").and_then(|s| s.split_once('(')).map(|v| v.0)
    }).collect::<Vec<_>>();
    if exported.len() != 1 || !identifier(exported[0]) {
        return Ok(None);
    }
    // These are not self-contained function moves: the syntax and
    // visibility would need different, separately audited rules.
    if original.lines().any(|line| line.trim_start().starts_with("#["))
        || original.contains("super::") || original.contains("self::")
        || original.contains("crate::")
    {
        return Ok(None);
    }
    Ok(Some((name.to_owned(), exported[0].to_owned(), header.len() + 1)))
}

fn line_byte(text: &str, line: usize, column: usize) -> Result<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text[start..].find('\n').ok_or("compiler line outside source")? + 1;
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
    message.strip_prefix("cannot find function \`")?
        .strip_suffix("\` in this scope")
        .filter(|name| identifier(name))
}

/// Pure planning boundary, tested without a language server or Git worktree.
fn preserve_bytes(
    path: &str,
    oid: &str,
    old_source: &str,
    current_source: &str,
    prior_edits: &Value,
    errors: &[Missing<'_>],
) -> Result<Option<Proposal>> {
    let edits = prior_edits.array_ref()?;
    if edits.len() != 1 { return Ok(None); }
    let change = &edits[0];
    if change.get("path")?.str()? != path { return Ok(None); }
    if change.get("original_blob")?.str()? != oid {
        return refuse("native extraction edit is not bound to original Git source blob");
    }
    let start = usize::try_from(change.get("start_byte")?.integer()?)
        .map_err(|_| "invalid native extraction start")?;
    let end = usize::try_from(change.get("end_byte")?.integer()?)
        .map_err(|_| "invalid native extraction end")?;
    if start >= end || end > old_source.len()
        || !old_source.is_char_boundary(start) || !old_source.is_char_boundary(end)
    {
        return refuse("native extracted range is not a valid UTF-8 source range");
    }
    let replacement = change.get("replacement")?.str()?;
    let Some((module, function, header_bytes)) =
        unchanged_move(&old_source[start..end], replacement)?
    else { return Ok(None); };
    let mut expected = old_source.to_owned();
    expected.replace_range(start..end, replacement);
    if expected != current_source {
        return refuse("native candidate is not exactly the original single-item source edit");
    }
    let module_begin = start + header_bytes;
    let module_end = start + replacement.len();
    let mut helpers = BTreeSet::new();
    for error in errors {
        if error.path != path { continue; }
        let pos = line_byte(current_source, error.line, error.column)?;
        if pos < module_begin || pos >= module_end { continue; }
        if error.code != "E0425" {
            return refuse("native extraction has unsupported unresolved names in the moved module");
        }
        let Some(helper) = missing_function(error.message) else {
            return refuse("unrecognized native extracted-module compiler diagnostic");
        };
        if helper == function
            || !current_source[pos..].starts_with(helper)
            || !current_source[pos + helper.len()..].starts_with('(')
            || !old_source.lines().any(|line| line.starts_with(&format!("fn {helper}("))
                || line.starts_with(&format!("pub fn {helper}(")))
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
    let imports = helpers.iter().map(|h| format!("    use super::{h};\n"))
        .collect::<String>();
    let export = format!("\npub use self::{module}::{function};");
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
        moved_function: function,
        helpers: helpers.into_iter().collect(),
    }))
}

pub(crate) fn propose(
    repo: &Path,
    base: &str,
    parent: &Value,
    errors: &[Missing<'_>],
) -> Result<Option<Proposal>> {
    if parent.get("action_title")?.str()? != "Extract Module"
        || parent.get("action_kind")?.str()? != "refactor.extract"
    {
        return Ok(None);
    }
    let changes = parent.get("changes")?.array_ref()?;
    if changes.len() != 1 { return Ok(None); }
    let path = changes[0].get("path")?.str()?;
    git::validate_path(path)?;
    if !path.ends_with(".rs") { return Ok(None); }
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
    let current_source = String::from_utf8(bytes).map_err(|_| "candidate Rust source is not UTF-8")?;
    preserve_bytes(
        path, &original.oid, &original_source, &current_source,
        parent.get("edits")?, errors,
    ).map(|proposal| proposal.map(|mut result| {
        // Translate original-blob anchors to the *committed candidate*
        // revision, because that is the exact input to the second edit.
        let edits = result.edits.array_ref().expect("constructed local edits");
        result.edits = Value::array(edits.iter().map(|item| obj(&[
            ("path", item.get("path").expect("local path").clone()),
            ("original_blob", Value::string(&current_entry.oid)),
            ("start_byte", item.get("start_byte").expect("local range").clone()),
            ("end_byte", item.get("end_byte").expect("local range").clone()),
            ("replacement", item.get("replacement").expect("local text").clone()),
        ])));
        result
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (String, String, Value) {
        let source = "pub fn helper() -> usize { 2 }\n\n/// documented\npub fn public() -> usize {\n    helper()\n}\n\npub fn caller() -> usize { public() }\n";
        let moved = "/// documented\npub fn public() -> usize {\n    helper()\n}";
        let replacement = "mod extracted {\n    /// documented\n    pub fn public() -> usize {\n        helper()\n    }\n}";
        let start = source.find(moved).unwrap();
        let edit = refactor_codegen::edit_operation(
            "src/semantic.rs", Some("old"), start, start+moved.len(), replacement
        );
        let mut candidate = source.to_owned();
        candidate.replace_range(start..start+moved.len(), replacement);
        (source.to_owned(), candidate, Value::array([edit]))
    }
    #[test]
    fn preserves_exact_function_body_and_parent_public_path() {
        let (before, candidate, edits)=fixture();
        let line=candidate.lines().position(|l| l.contains("        helper()")).unwrap();
        let error=Missing{
            path:"src/semantic.rs",line,column:8,code:"E0425",
            message:"cannot find function \`helper\` in this scope",
        };
        let patch=preserve_bytes(
            "src/semantic.rs","old",&before,&candidate,&edits,&[error]
        ).unwrap().unwrap();
        let after=&patch.outputs["src/semantic.rs"];
        assert!(after.contains("    use super::helper;\n"));
        assert!(after.contains("pub use self::extracted::public;"));
        assert!(after.contains("pub fn public() -> usize {\n        helper()\n    }"));
        assert_eq!(patch.moved_function,"public");
        assert_eq!(patch.helpers,vec!["helper"]);
        assert_eq!(refactor_codegen::replay_edits(&candidate,&patch.edits,"src/semantic.rs").unwrap(),*after);
    }
    #[test]
    fn refuses_mutated_function_or_unsupported_diagnostic() {
        let (before,candidate,edits)=fixture();
        let corrupt=candidate.replace("        helper()", "        999");
        assert!(preserve_bytes("src/semantic.rs","old",&before,&corrupt,&edits,&[]).is_err());
        let line=candidate.lines().position(|l| l.contains("        helper()")).unwrap();
        let e=Missing{path:"src/semantic.rs",line,column:8,code:"E0433",message:"bad"};
        assert!(preserve_bytes("src/semantic.rs","old",&before,&candidate,&edits,&[e]).is_err());
        let e=Missing{path:"src/semantic.rs",line,column:8,code:"E0425",
            message:"cannot find function \`unknown\` in this scope"};
        assert!(preserve_bytes("src/semantic.rs","old",&before,&candidate,&edits,&[e]).is_err());
    }
}
