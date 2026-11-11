//! Read-only discovery of Git source inputs for the ordinary CLI.
//! No build commands, source code or checkout modification are performed here.
use crate::git;
use crate::json::{Value, obj};
use crate::{Result, doctor, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Discovery {
    pub root: PathBuf,
    pub commit: String,
    pub tree: String,
    pub roots: Vec<String>,
    pub index_path: PathBuf,
    pub source_files: usize,
    pub root_level_files: usize,
    pub language_counts: BTreeMap<&'static str, usize>,
}

/// Select all first-level directories containing supported tracked source,
/// not guessed 'src' and 'tests' paths. Top-level files are reported as
/// excluded because the existing module indexer requires a rooted directory.
fn source_layout(entries: &[git::Entry]) -> (Vec<String>, usize, usize, BTreeMap<&'static str, usize>) {
    let mut roots = BTreeSet::new();
    let mut included = 0usize;
    let mut root_level = 0usize;
    let mut languages = BTreeMap::new();
    for entry in entries {
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            continue;
        }
        let language = if entry.path.ends_with(".py") {
            "python"
        } else if entry.path.ends_with(".js")
            || entry.path.ends_with(".cjs")
            || entry.path.ends_with(".mjs")
            || entry.path.ends_with(".ts")
        {
            "javascript"
        } else {
            continue;
        };
        // The current indexer intentionally excludes installed packages.
        if entry.path.split('/').any(|p| p == "node_modules" || p == ".venv") {
            continue;
        }
        let Some((root, _)) = entry.path.split_once('/') else {
            root_level += 1;
            continue;
        };
        roots.insert(root.to_owned());
        included += 1;
        *languages.entry(language).or_insert(0usize) += 1;
    }
    (roots.into_iter().collect(), included, root_level, languages)
}

fn cache_path(root: &Path) -> PathBuf {
    let home = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(env::temp_dir);
    let dir = env::var_os("EXCESSION_CACHE_DIR").map(PathBuf::from).unwrap_or(home.join("excession"));
    let id = crate::digest::sha256(root.to_string_lossy().as_bytes());
    dir.join(&id[..24]).join("index.sqlite")
}

pub fn inspect(path: &Path, rev: &str) -> Result<Discovery> {
    let root = PathBuf::from(git::text(path, &["rev-parse", "--show-toplevel"])?);
    git::verify_repo(&root)?;
    let commit = git::rev(&root, rev)?;
    if git::text(&root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("repository discovery requires a commit revision");
    }
    let tree = git::rev(&root, &format!("{commit}^{{tree}}"))?;
    let entries = git::entries(&root, &commit)?;
    let (roots, source_files, root_level_files, language_counts) = source_layout(&entries);
    Ok(Discovery {
        index_path: cache_path(&root),
        root, commit, tree, roots, source_files, root_level_files, language_counts,
    })
}

impl Discovery {
    pub fn report(&self) -> Result<Value> {
        let capabilities = doctor::report();
        let caps = capabilities.get("capabilities")?;
        let mut adapters = Vec::new();
        let mut gaps = Vec::new();
        for (lang, cap) in [("python", "python_index"), ("javascript", "javascript_index")] {
            let found = caps.get(cap)?.bool()?;
            adapters.push(obj(&[
                ("name", Value::string(lang)),
                ("availability", Value::string(if found { "AVAILABLE" } else { "UNAVAILABLE" })),
            ]));
            if !found && self.language_counts.contains_key(lang) {
                gaps.push(obj(&[
                    ("reason", Value::string("MISSING_ADAPTER")),
                    ("detail", Value::string(format!("{lang} analyzer is not installed or ready"))),
                ]));
            }
        }
        if self.root_level_files > 0 {
            gaps.push(obj(&[
                ("reason", Value::string("EXCLUDED_SCOPE")),
                ("detail", Value::string(format!(
                    "{} tracked root-level source file(s) cannot be indexed by the current module-root API",
                    self.root_level_files
                ))),
            ]));
        }
        if self.roots.is_empty() {
            gaps.push(obj(&[
                ("reason", Value::string("EXCLUDED_SCOPE")),
                ("detail", Value::string("No supported source files below a first-level directory")),
            ]));
        }
        let status = if self.roots.is_empty() { "UNKNOWN" } else { "PARTIAL" };
        Ok(obj(&[
            ("schema", Value::string("excession.discovery.v1")),
            ("source_commit", Value::string(&self.commit)),
            ("source_tree", Value::string(&self.tree)),
            ("repository_path", Value::string(self.root.to_string_lossy())),
            ("roots", Value::array(self.roots.iter().map(Value::string).collect())),
            ("adapters", Value::array(adapters)),
            ("index_path", Value::string(self.index_path.to_string_lossy())),
            ("coverage", obj(&[
                ("status", Value::string(status)),
                ("analyzed_sources", Value::number(0)),
                ("gaps", Value::array(gaps)),
            ])),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> git::Entry {
        git::Entry { path: path.into(), mode: "100644".into(), kind: "blob".into(), oid: "0".repeat(40) }
    }

    #[test]
    fn discovers_all_tracked_source_roots_not_guessed_defaults() {
        let (roots, n, root_level, langs) = source_layout(&[
            entry("lib/impl.py"), entry("examples/demo.ts"), entry("tests/test_impl.py"),
            entry("script.py"), entry("node_modules/a.js"), entry(".venv/lib/b.py"),
        ]);
        assert_eq!(roots, ["examples", "lib", "tests"]);
        assert_eq!(n, 3);
        assert_eq!(root_level, 1);
        assert_eq!(langs["python"], 2);
        assert_eq!(langs["javascript"], 1);
    }

    #[test]
    fn excludes_git_symlinks_from_supported_source_inventory() {
        let mut link = entry("lib/link.py");
        link.mode = "120000".into();
        let (roots, count, _, _) = source_layout(&[link]);
        assert!(roots.is_empty());
        assert_eq!(count, 0);
    }
}
