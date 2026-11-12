//! Pure path-to-module naming for indexed Python and JavaScript sources.
//! This module does not parse source or read mutable checkout state.

// Logical language-module names are derived from Git paths before indexing.
// This pure conversion is shared by semantic preparation, revision indexing,
// and incremental reuse without requiring dependencies on either orchestrator.
pub fn module_name(path: &str, roots: &[&str]) -> Option<(String, bool)> {
    if path
        .split('/')
        .any(|part| matches!(part, "node_modules" | ".venv"))
    {
        return None;
    }
    for &root in roots {
        // "." is an explicit whole-repository root. It supports tracked
        // top-level modules without inventing a path prefix.
        if root != "." {
            let prefix = format!("{}/", root.trim_end_matches('/').replace('.', "/"));
            if !path.starts_with(&prefix) {
                continue;
            }
        }
        let stem = if let Some(p) = path.strip_suffix(".py") {
            p.to_owned()
        } else if let Some(p) = path.strip_suffix(".js") {
            p.to_owned()
        } else if let Some(p) = path.strip_suffix(".mjs") {
            p.to_owned() + ".mjs"
        } else if let Some(p) = path.strip_suffix(".cjs") {
            p.to_owned() + ".cjs"
        } else if let Some(p) = path.strip_suffix(".ts") {
            p.to_owned() + ".ts"
        } else {
            continue;
        };
        let module = stem.replace('/', ".");
        let name = module
            .strip_suffix(".__init__")
            .or_else(|| module.strip_suffix(".index"))
            .unwrap_or(&module);
        let is_test = path.starts_with("tests/")
            || path.starts_with("h11/tests/")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|p| p.starts_with("test_"));
        return Some((name.to_owned(), is_test));
    }
    None
}

/// Convert a Git-rooted module identity to a declared Python import name.
/// Historical callers without an import base keep their old module names.
pub fn module_name_with_import_base(
    path: &str,
    roots: &[&str],
    import_base: Option<&str>,
) -> Option<(String, bool)> {
    let (name, is_test) = module_name(path, roots)?;
    let Some(import_base) = import_base else {
        return Some((name, is_test));
    };
    let prefix = format!("{}.", import_base.replace('/', "."));
    Some((name.strip_prefix(&prefix)?.to_owned(), is_test))
}
