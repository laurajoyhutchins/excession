//! Read-only discovery of Git source inputs for the ordinary CLI.
//! No build commands, source code or checkout modification are performed here.
use crate::git;
use crate::json::{Value, obj};
use crate::source_capabilities;
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
    /// (tracked source count, static-module-indexable count), by detected language.
    pub source_languages: BTreeMap<String, (usize, usize)>,
}

/// Inventory all language-detected tracked files, but select roots only for
/// languages with a real static-module producer. Source detection is not
/// evidence of resolved dependencies or installed parser availability.
fn source_layout(
    entries: &[git::Entry],
) -> (
    Vec<String>,
    usize,
    usize,
    BTreeMap<&'static str, usize>,
    BTreeMap<String, (usize, usize)>,
) {
    let mut roots = BTreeSet::new();
    let mut included = 0usize;
    let mut root_level = 0usize;
    let mut languages = BTreeMap::new();
    let mut source_languages = BTreeMap::<String, (usize, usize)>::new();
    for entry in entries {
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            continue;
        }
        // Exclude known installed package trees from the product source census;
        // they remain present in the authoritative Git tree.
        if source_capabilities::excluded_from_static_modules(&entry.path) {
            continue;
        }
        let Some(detected) = source_capabilities::detected_language(&entry.path) else {
            continue;
        };
        let count = source_languages.entry(detected).or_default();
        count.0 += 1;
        let Some(producer) = source_capabilities::static_module_producer(&entry.path) else {
            continue;
        };
        count.1 += 1;
        let language = producer.name;
        let Some((root, _)) = entry.path.split_once('/') else {
            root_level += 1;
            *languages.entry(language).or_insert(0usize) += 1;
            continue;
        };
        roots.insert(root.to_owned());
        included += 1;
        *languages.entry(language).or_insert(0usize) += 1;
    }
    if root_level > 0 {
        // The module indexer can use "." to include both top-level files
        // and nested modules. Reporting an exclusion here would be incorrect.
        roots.clear();
        roots.insert(".".to_owned());
        included += root_level;
        root_level = 0;
    }
    (
        roots.into_iter().collect(),
        included,
        root_level,
        languages,
        source_languages,
    )
}

fn cache_path(root: &Path) -> PathBuf {
    let home = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(env::temp_dir);
    let dir = env::var_os("EXCESSION_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or(home.join("excession"));
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
    let (roots, source_files, root_level_files, language_counts, source_languages) =
        source_layout(&entries);
    Ok(Discovery {
        index_path: cache_path(&root),
        root,
        commit,
        tree,
        roots,
        source_files,
        root_level_files,
        language_counts,
        source_languages,
    })
}

impl Discovery {
    pub fn report(&self) -> Result<Value> {
        let capabilities = doctor::report();
        let caps = capabilities.get("capabilities")?;
        let mut adapters = Vec::new();
        let mut gaps = Vec::new();
        for producer in source_capabilities::STATIC_MODULE_PRODUCERS {
            let lang = producer.name;
            let found = caps.get(producer.capability)?.bool()?;
            adapters.push(obj(&[
                ("name", Value::string(lang)),
                (
                    "availability",
                    Value::string(if found { "AVAILABLE" } else { "UNAVAILABLE" }),
                ),
            ]));
            if !found && self.language_counts.contains_key(lang) {
                gaps.push(obj(&[
                    ("reason", Value::string("MISSING_ADAPTER")),
                    (
                        "detail",
                        Value::string(format!("{lang} analyzer is not installed or ready")),
                    ),
                ]));
            }
        }
        for (language, &(tracked, indexable)) in &self.source_languages {
            if tracked > indexable {
                gaps.push(obj(&[
                    ("reason", Value::string("UNSUPPORTED_LANGUAGE")),
                    (
                        "detail",
                        Value::string(format!(
                            "{language}: {} detected source file(s) have no static-module producer; syntax or native observations require a separate configured workflow",
                            tracked - indexable
                        )),
                    ),
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
                (
                    "detail",
                    Value::string("No supported source files below a first-level directory"),
                ),
            ]));
        }
        let status = if self.roots.is_empty() {
            "UNKNOWN"
        } else {
            "PARTIAL"
        };
        Ok(obj(&[
            ("schema", Value::string("excession.discovery.v1")),
            ("source_commit", Value::string(&self.commit)),
            ("source_tree", Value::string(&self.tree)),
            (
                "repository_path",
                Value::string(self.root.to_string_lossy()),
            ),
            ("roots", Value::array(self.roots.iter().map(Value::string))),
            (
                "source_languages",
                Value::array(self.source_languages.iter().map(
                    |(language, (tracked, indexable))| {
                        obj(&[
                            ("language", Value::string(language)),
                            ("tracked_sources", Value::number(*tracked)),
                            ("indexable_sources", Value::number(*indexable)),
                        ])
                    },
                )),
            ),
            ("adapters", Value::array(adapters)),
            (
                "index_path",
                Value::string(self.index_path.to_string_lossy()),
            ),
            (
                "coverage",
                obj(&[
                    ("status", Value::string(status)),
                    ("analyzed_sources", Value::number(0)),
                    ("gaps", Value::array(gaps)),
                ]),
            ),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> git::Entry {
        git::Entry {
            path: path.into(),
            mode: "100644".into(),
            kind: "blob".into(),
            oid: "0".repeat(40),
        }
    }

    #[test]
    fn discovers_all_tracked_source_roots_not_guessed_defaults() {
        let (roots, n, root_level, langs, census) = source_layout(&[
            entry("lib/impl.py"),
            entry("examples/demo.ts"),
            entry("tests/test_impl.py"),
            entry("script.py"),
            entry("node_modules/a.js"),
            entry(".venv/lib/b.py"),
        ]);
        assert_eq!(roots, ["."]);
        assert_eq!(n, 4);
        assert_eq!(root_level, 0);
        assert_eq!(langs["python"], 3);
        assert_eq!(langs["javascript"], 1);
        assert_eq!(
            census.values().map(|(tracked, _)| tracked).sum::<usize>(),
            4
        );
    }

    #[test]
    fn inventories_other_languages_without_pretending_to_index_them() {
        let (roots, count, _, langs, census) = source_layout(&[
            entry("src/lib.rs"),
            entry("cmd/server.go"),
            entry("native/main.c"),
            entry("src/app.py"),
            entry("node_modules/lib.rs"),
        ]);
        assert_eq!(roots, ["src"]);
        assert_eq!(count, 1);
        assert_eq!(langs["python"], 1);
        for lang in ["rust", "go", "c"] {
            assert!(
                census
                    .get(lang)
                    .is_some_and(|(total, indexable)| *total == 1 && *indexable == 0)
            );
        }
    }

    #[test]
    fn excludes_git_symlinks_from_supported_source_inventory() {
        let mut link = entry("lib/link.py");
        link.mode = "120000".into();
        let (roots, count, _, _, _) = source_layout(&[link]);
        assert!(roots.is_empty());
        assert_eq!(count, 0);
    }
}
