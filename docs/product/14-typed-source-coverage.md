# Review and refactor: source coverage is data, not a string match

Previously, `impact_query::observe` caught an English diagnostic from `analyze::run` using `starts_with("no supported Python/JavaScript...")`. A wording change could turn an ordinary unsupported-only revision into a fatal impact error, or accidentally reinterpret a different error as successful absence.

`analyze::run_if_supported` now returns `Result<Option<(Report, CachePath)>>`. `None` means the discovered revision has no supported source roots; actual capability, Git, input, and SQLite errors remain `Err`. `analyze::run` still returns the existing actionable CLI error on `None`.

`impact` uses this typed boundary instead of matching diagnostics. A changed documentation-only revision now reports `EXCLUDED_SCOPE` alongside `UNSUPPORTED_LANGUAGE`, instead of misleadingly blaming a missing adapter. No source-specific behavior or ontology support is inferred.

The added real Git CLI integration test checks both entrypoints and the no-test-skipping safety contract. This reuses the existing indexer, discovery, and report schemas without adding another dispatcher or evidence store.
