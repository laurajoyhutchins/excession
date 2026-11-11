//! Conservative candidate relationships over authenticated Linux syntax facts.
//! Matching a Git path or Kconfig name does not prove an active build edge.
use crate::Result;
use crate::json::{Value, obj};
use std::collections::{BTreeMap, BTreeSet};

fn joined(base: &str, target: &str) -> Option<String> {
    let mut parts = base
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    for piece in target.split('/') {
        match piece {
            "" | "." => (),
            ".." => {
                parts.pop()?;
            }
            segment if segment.contains('\\') || segment.contains('\0') => return None,
            segment => parts.push(segment),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn config_names(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut names = BTreeSet::new();
    let mut at = 0;
    while at + 7 <= bytes.len() {
        let boundary = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        if boundary && bytes[at..].starts_with(b"CONFIG_") {
            let start = at + 7;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start {
                names.insert(text[start..end].to_owned());
            }
            at = end;
        } else {
            at += 1;
        }
    }
    names
}

fn link(
    from: &str,
    from_oid: &str,
    to: &str,
    to_oid: &str,
    fact: &Value,
    kind: &str,
    status: &str,
) -> Result<Value> {
    Ok(obj(&[
        ("source_path", Value::string(from)),
        ("source_blob", Value::string(from_oid)),
        ("target_path", Value::string(to)),
        ("target_blob", Value::string(to_oid)),
        ("line", fact.get("line")?.clone()),
        ("condition", fact.get("condition")?.clone()),
        ("kind", Value::string(kind)),
        ("status", Value::string(status)),
    ]))
}

fn append_path(
    edges: &mut Vec<Value>,
    tracked: &BTreeMap<String, String>,
    from: &str,
    from_oid: &str,
    fact: &Value,
    target: Option<String>,
    kind: &str,
) -> Result<()> {
    if let Some(path) = target
        && let Some(oid) = tracked.get(&path)
    {
        edges.push(link(
            from,
            from_oid,
            &path,
            oid,
            fact,
            kind,
            "GIT_PATH_MATCH_ONLY",
        )?);
    }
    Ok(())
}

/// Relations that are mechanically supported by matching names to paths in
/// one pinned Git tree. Build-configuration semantics remain unresolved.
pub fn derive(sources: &[Value], tracked: &BTreeMap<String, String>) -> Result<Vec<Value>> {
    let mut symbols = BTreeMap::<String, Vec<(String, String)>>::new();
    for source in sources {
        let path = source.get("source_path")?.str()?;
        let oid = source.get("source_blob")?.str()?;
        for fact in source.get("facts")?.array_ref()? {
            if fact.get("kind")?.str()? == "kconfig_symbol" {
                symbols
                    .entry(fact.get("subject")?.str()?.to_owned())
                    .or_default()
                    .push((path.to_owned(), oid.to_owned()));
            }
        }
    }

    let mut edges = Vec::new();
    for source in sources {
        let path = source.get("source_path")?.str()?;
        let oid = source.get("source_blob")?.str()?;
        let directory = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        for fact in source.get("facts")?.array_ref()? {
            let kind = fact.get("kind")?.str()?;
            let target = fact.get("target")?.str()?;
            let condition = fact.get("condition")?.str()?;
            match kind {
                "c_include" => {
                    let base = if fact.get("subject")?.str()? == "quoted" {
                        directory
                    } else {
                        "include"
                    };
                    append_path(
                        &mut edges,
                        tracked,
                        path,
                        oid,
                        fact,
                        joined(base, target),
                        "candidate-header-include",
                    )?;
                }
                "build_object" if target.ends_with(".o") => {
                    let stem = target
                        .strip_suffix(".o")
                        .ok_or("object suffix disappeared")?;
                    append_path(
                        &mut edges,
                        tracked,
                        path,
                        oid,
                        fact,
                        joined(directory, &format!("{stem}.c")),
                        "candidate-object-source",
                    )?;
                    for config in config_names(condition) {
                        if let Some(declarations) = symbols.get(&config) {
                            for (decl_path, decl_oid) in declarations {
                                edges.push(link(
                                    path,
                                    oid,
                                    decl_path,
                                    decl_oid,
                                    fact,
                                    "candidate-config-declaration",
                                    "CONFIG_NAME_MATCH_ONLY",
                                )?);
                            }
                        }
                    }
                }
                "kconfig_source" if !target.contains('$') => {
                    let base = if fact.get("subject")?.str()?.starts_with('r') {
                        directory
                    } else {
                        ""
                    };
                    append_path(
                        &mut edges,
                        tracked,
                        path,
                        oid,
                        fact,
                        joined(base, target),
                        "candidate-kconfig-source",
                    )?;
                }
                _ => {}
            }
        }
    }
    let mut seen = BTreeSet::new();
    edges.retain(|edge| seen.insert(edge.canonical()));
    edges.sort_by_key(Value::canonical);
    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_quote_includes_without_escaping_tree() {
        assert_eq!(
            joined("drivers/rtc", "local.h").as_deref(),
            Some("drivers/rtc/local.h")
        );
        assert_eq!(
            joined("drivers/rtc", "../base.h").as_deref(),
            Some("drivers/base.h")
        );
        assert!(joined("", "../outside.h").is_none());
        assert!(joined("drivers", "x\\bad").is_none());
    }

    #[test]
    fn configuration_names_are_distinct_and_exact() {
        assert_eq!(
            config_names("(CONFIG_FOO) || CONFIG_BAR && CONFIG_FOO && NOTCONFIG_FAKE"),
            BTreeSet::from(["FOO".to_owned(), "BAR".to_owned()])
        );
    }
}
