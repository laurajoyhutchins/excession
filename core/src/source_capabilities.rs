//! Language-native source producer capabilities for the ordinary static index.
//! These are *capabilities*, not a declaration that all detected languages
//! have an installed parser, semantic resolver, or complete dependency graph.
use tree_sitter_language_pack::detect_language_from_path;

#[derive(Debug)]
pub struct StaticModuleProducer {
    pub name: &'static str,
    pub capability: &'static str,
    extensions: &'static [&'static str],
}

/// Single authority for the legacy static-module index. New producers must
/// supply real observations before being added; syntax detection alone is not
/// sufficient to claim static-import support.
pub const STATIC_MODULE_PRODUCERS: &[StaticModuleProducer] = &[
    StaticModuleProducer {
        name: "python",
        capability: "python_index",
        extensions: &[".py"],
    },
    StaticModuleProducer {
        name: "javascript",
        capability: "javascript_index",
        extensions: &[".js", ".cjs", ".mjs", ".ts"],
    },
];

/// Do not mistake installed package trees for project-owned modules.
/// Additional exclusion policies belong to producers, not global Git identity.
pub fn excluded_from_static_modules(path: &str) -> bool {
    path.split('/')
        .any(|component| matches!(component, "node_modules" | ".venv"))
}

pub fn static_module_producer(path: &str) -> Option<&'static StaticModuleProducer> {
    if excluded_from_static_modules(path) {
        return None;
    }
    STATIC_MODULE_PRODUCERS.iter().find(|producer| {
        producer
            .extensions
            .iter()
            .any(|extension| path.ends_with(extension))
    })
}

/// A language label is an inventory observation only. This does not invoke a
/// parser, require a cached grammar, or certify any semantic relationship.
pub fn detected_language(path: &str) -> Option<String> {
    detect_language_from_path(path)
        .map(str::to_owned)
        .or_else(|| static_module_producer(path).map(|producer| producer.name.to_owned()))
}

/// Best-effort test *candidate* for agent context, never proof of test coverage.
pub fn test_candidate(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|component| component == "tests" || component == "test")
        || file.starts_with("test_")
        || file.ends_with("_test.rs")
        || file.ends_with("_test.py")
        || file.ends_with("_test.go")
        || [
            ".test.ts",
            ".spec.ts",
            ".test.js",
            ".spec.js",
            ".test.tsx",
            ".spec.tsx",
            ".test.jsx",
            ".spec.jsx",
        ]
        .iter()
        .any(|suffix| file.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_indexing_is_a_producer_capability_not_a_language_guess() {
        assert_eq!(
            static_module_producer("src/main.py").unwrap().name,
            "python"
        );
        assert_eq!(
            static_module_producer("web/index.ts").unwrap().name,
            "javascript"
        );
        assert_eq!(static_module_producer("web/view.tsx").map(|p| p.name), None);
        assert_eq!(static_module_producer("src/main.rs").map(|p| p.name), None);
        assert_eq!(
            static_module_producer("node_modules/pkg/a.js").map(|p| p.name),
            None
        );
        assert_eq!(
            static_module_producer(".venv/lib/a.py").map(|p| p.name),
            None
        );
    }

    #[test]
    fn tests_are_candidate_metadata_not_h11_specific() {
        assert!(test_candidate("someproject/tests/unit.py"));
        assert!(test_candidate("packages/api/foo_test.go"));
        assert!(test_candidate("src/widget.spec.tsx"));
        assert!(!test_candidate("src/testing_utils.py"));
        assert!(!test_candidate("src/module.rs"));
    }

    #[test]
    fn native_language_detection_does_not_imply_module_indexing() {
        assert_eq!(detected_language("src/lib.rs").as_deref(), Some("rust"));
        assert_eq!(detected_language("cmd/main.go").as_deref(), Some("go"));
        assert!(static_module_producer("src/lib.rs").is_none());
    }
}
