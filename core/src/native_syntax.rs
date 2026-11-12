//! Optional offline Tree-sitter observations using the upstream Rust language pack.
//! Rust owns the report format and resource bounds; this does not claim semantic completeness.
//! The upstream crate's auto-download feature is intentionally disabled.
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;
use tree_sitter_language_pack::{
    LanguageRegistry, ProcessConfig, StructureItem, StructureKind, detect_language_from_path,
};

const PARSER_VERSION: &str = "1.25.0";
const MAX_FILES: usize = 32;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_OBSERVATIONS: usize = 2048;
const MAX_STRUCTURE_DEPTH: usize = 64;

static REGISTRY: LazyLock<LanguageRegistry> = LazyLock::new(|| {
    // The environment is a trusted setup choice, never a path derived from
    // the untrusted repository. No grammar is fetched during a scan.
    if let Some(path) = std::env::var_os("TREE_SITTER_LANGUAGE_PACK_LIBS_DIR") {
        return LanguageRegistry::with_libs_dir(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("TREE_SITTER_LANGUAGE_PACK_CACHE_DIR") {
        return LanguageRegistry::with_libs_dir(
            PathBuf::from(path)
                .join("tree-sitter-language-pack")
                .join(format!("v{PARSER_VERSION}"))
                .join("libs"),
        );
    }
    LanguageRegistry::new()
});

fn observation(
    path: &str,
    language: &str,
    status: &str,
    structure: Vec<Value>,
    imports: Vec<Value>,
    diagnostics: Vec<Value>,
) -> Value {
    obj(&[
        ("path", Value::string(path)),
        ("language", Value::string(language)),
        ("status", Value::string(status)),
        ("structure", Value::array(structure)),
        ("imports", Value::array(imports)),
        ("diagnostics", Value::array(diagnostics)),
    ])
}

fn line_number(line: usize) -> Result<usize> {
    line.checked_add(1)
        .ok_or("source line number overflow".to_owned())
}

fn structure_item(item: &StructureItem, depth: usize, budget: &mut usize) -> Result<Value> {
    if depth >= MAX_STRUCTURE_DEPTH || *budget == 0 {
        return refuse("Tree-sitter structural observation exceeds resource budget");
    }
    *budget -= 1;
    let kind = match &item.kind {
        StructureKind::Other(name) => name.to_ascii_lowercase(),
        other => format!("{other:?}").to_ascii_lowercase(),
    };
    let mut children = Vec::new();
    for child in &item.children {
        children.push(structure_item(child, depth + 1, budget)?);
    }
    Ok(obj(&[
        ("kind", Value::string(kind)),
        ("name", Value::string(item.name.as_deref().unwrap_or(""))),
        ("line", Value::number(line_number(item.span.start_line)?)),
        ("start_byte", Value::number(item.span.start_byte)),
        ("end_byte", Value::number(item.span.end_byte)),
        ("children", Value::array(children)),
    ]))
}

/// Exact response shape of the previous Python bridge, from the same language-pack
/// upstream engine and grammar cache. Not a resolver or a source-coverage proof.
pub fn parse(entries: &[Value]) -> Result<Value> {
    if entries.len() > MAX_FILES {
        return refuse("Tree-sitter parser accepts at most 32 source records");
    }
    let mut seen = BTreeSet::new();
    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.entries()?.len() != 2 {
            return refuse("Tree-sitter record requires path and content only");
        }
        let path = entry.get("path")?.str()?;
        let text = entry.get("content")?.str()?;
        if path.is_empty() || !seen.insert(path.to_owned()) {
            return refuse("Tree-sitter source path empty or duplicated");
        }
        if text.len() > MAX_SOURCE_BYTES {
            return refuse("Tree-sitter source exceeds 512 KiB");
        }
        let Some(language) = detect_language_from_path(path) else {
            records.push(observation(
                path,
                "",
                "UNSUPPORTED_LANGUAGE",
                vec![],
                vec![],
                vec![],
            ));
            continue;
        };
        if !REGISTRY.has_parser(language) {
            records.push(observation(
                path,
                language,
                "GRAMMAR_NOT_INSTALLED",
                vec![],
                vec![],
                vec![],
            ));
            continue;
        }
        let mut config = ProcessConfig::new(language);
        config.exports = false;
        config.diagnostics = true;
        config.max_source_bytes = Some(MAX_SOURCE_BYTES);
        config.parse_timeout_ms = Some(1000);
        let parsed = match REGISTRY.process(text, &config) {
            Ok(parsed) => parsed,
            Err(_) => {
                records.push(observation(
                    path,
                    language,
                    "PARSER_ERROR",
                    vec![],
                    vec![],
                    vec![],
                ));
                continue;
            }
        };
        let converted = (|| -> Result<(Vec<Value>, Vec<Value>, Vec<Value>)> {
            let mut budget = MAX_OBSERVATIONS;
            let mut structure = Vec::new();
            for item in &parsed.structure {
                structure.push(structure_item(item, 0, &mut budget)?);
            }
            let mut imports = Vec::new();
            for item in &parsed.imports {
                if budget == 0 {
                    return refuse("Tree-sitter import count exceeds resource budget");
                }
                budget -= 1;
                imports.push(obj(&[
                    ("source", Value::string(&item.source)),
                    ("line", Value::number(line_number(item.span.start_line)?)),
                    ("wildcard", Value::Bool(item.is_wildcard)),
                ]));
            }
            let mut diagnostics = Vec::new();
            for item in &parsed.diagnostics {
                if budget == 0 {
                    return refuse("Tree-sitter diagnostic count exceeds resource budget");
                }
                budget -= 1;
                diagnostics.push(obj(&[
                    ("line", Value::number(line_number(item.span.start_line)?)),
                    (
                        "message",
                        Value::string(item.message.chars().take(512).collect::<String>()),
                    ),
                ]));
            }
            Ok((structure, imports, diagnostics))
        })();
        match converted {
            Ok((structure, imports, diagnostics)) => {
                let status = if parsed.metrics.error_count == 0 {
                    "PARSED"
                } else {
                    "PARSED_WITH_ERRORS"
                };
                records.push(observation(
                    path,
                    language,
                    status,
                    structure,
                    imports,
                    diagnostics,
                ));
            }
            Err(_) => records.push(observation(
                path,
                language,
                "RESOURCE_LIMIT",
                vec![],
                vec![],
                vec![],
            )),
        }
    }
    Ok(obj(&[
        ("parser_version", Value::string(PARSER_VERSION)),
        ("sources", Value::array(records)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_duplicate_source_and_oversized_input() {
        let one = obj(&[
            ("path", Value::string("a.rs")),
            ("content", Value::string("fn main() {}")),
        ]);
        assert!(parse(&[one.clone(), one]).is_err());
        let huge = obj(&[
            ("path", Value::string("a.rs")),
            ("content", Value::string("x".repeat(MAX_SOURCE_BYTES + 1))),
        ]);
        assert!(parse(&[huge]).is_err());
    }

    #[test]
    fn unknown_extension_keeps_unsupported_coverage_explicit() {
        let item = obj(&[
            ("path", Value::string("a.opaque")),
            ("content", Value::string("anything")),
        ]);
        let parsed = parse(&[item]).unwrap();
        let row = &parsed.get("sources").unwrap().array_ref().unwrap()[0];
        assert_eq!(
            row.get("status").unwrap().str().unwrap(),
            "UNSUPPORTED_LANGUAGE"
        );
        assert!(
            row.get("structure")
                .unwrap()
                .array_ref()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn offline_rust_grammar_does_not_invent_parse_success() {
        let item = obj(&[
            ("path", Value::string("src/main.rs")),
            ("content", Value::string("fn main() {}\n")),
        ]);
        let parsed = parse(&[item]).unwrap();
        let row = &parsed.get("sources").unwrap().array_ref().unwrap()[0];
        let status = row.get("status").unwrap().str().unwrap();
        assert!(matches!(
            status,
            "PARSED" | "PARSED_WITH_ERRORS" | "GRAMMAR_NOT_INSTALLED" | "PARSER_ERROR"
        ));
        if status == "GRAMMAR_NOT_INSTALLED" {
            assert!(row.get("imports").unwrap().array_ref().unwrap().is_empty());
        }
    }
}
