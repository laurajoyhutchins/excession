//! Pure path-to-module naming for indexed Python and JavaScript sources.
//! This module does not parse source or read mutable checkout state.

// Logical language-module names are derived from Git paths before indexing.
// This pure conversion is shared by semantic preparation, revision indexing,
// and incremental reuse without requiring dependencies on either orchestrator.
pub fn module_name(path: &str, roots: &[&str]) -> Option<(String, bool)> {
    crate::source_capabilities::static_module_producer(path)?;
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
            p.to_owned() + ".js"
        } else if let Some(p) = path.strip_suffix(".jsx") {
            p.to_owned() + ".jsx"
        } else if let Some(p) = path.strip_suffix(".tsx") {
            p.to_owned() + ".tsx"
        } else if let Some(p) = path.strip_suffix(".mts") {
            p.to_owned() + ".mts"
        } else if let Some(p) = path.strip_suffix(".cts") {
            p.to_owned() + ".cts"
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
        let is_test = crate::source_capabilities::test_candidate(path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_python_and_javascript_paths_never_alias() {
        let roots = &["."];
        let paths = [
            "src/app.py", "src/app.js", "src/app.jsx", "src/app.ts",
            "src/app.tsx", "src/app.mjs", "src/app.cjs", "src/app.mts",
            "src/app.cts",
        ];
        let names = paths
            .iter()
            .map(|path| module_name(path, roots).unwrap().0)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), paths.len());
        assert_eq!(module_name("src/app.py", roots).unwrap().0, "src.app");
        assert_eq!(module_name("src/app.js", roots).unwrap().0, "src.app.js");
        assert_eq!(module_name("src/app.tsx", roots).unwrap().0, "src.app.tsx");
    }

    #[test]
    fn python_package_name_does_not_alias_js_index_module() {
        assert_eq!(module_name("pkg/__init__.py", &["."]).unwrap().0, "pkg");
        assert_eq!(module_name("pkg/index.js", &["."]).unwrap().0, "pkg.index.js");
    }
}
