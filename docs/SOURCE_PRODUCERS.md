# Source producer boundary

Excession's default static-module index is a **specific observation producer**,
not a universal definition of source code. Git remains authoritative for all
tracked paths and blobs. The producer inventory in
`core/src/source_capabilities.rs` declares the existing Python and JavaScript
module indexers and their tool-availability capabilities.

`excession discover . --json` reports `source_languages`: syntax-recognized
tracked sources and the subset eligible for the ordinary static-module
indexer. Other languages are reported as `UNSUPPORTED_LANGUAGE` coverage
gaps. Detection is a source census, not a successful parse or resolved
dependency. Excluded installed dependencies are still present in Git but not
counted as project-owned modules.

`excession analyze` and `excession impact` consume that same static-module
producer registry. Neither should independently guess supported languages
from extensions. No adapter is silently skipped when a supported language
is selected but its required executable is missing.

## Adding a real semantic producer

1. Implement native observations using the language's parser, SCIP indexer,
   compiler, build system, or scanner. Preserve native names and semantics.
2. Bind each observation to pinned source identity, producer and configuration,
   with explicit unresolved/unsupported coverage and replay.
3. Register the producer only after it can supply actual, testable
   relationships. Merely recognizing the extension is not sufficient.
4. Add mixed-language Git fixtures and independent semantic witnesses.

This change does not make `analyze` resolve Rust, Go, C, or cross-language
imports. Existing `syntax`, `semantic`, `scip`, Clang, CMake, and Kbuild
paths remain separately available; integrating their observations into the
ordinary analysis and impact query is further work.

Test candidates are heuristic metadata. Source/test and build/test
relationships should ultimately come from native target and execution
evidence, not filename patterns. All unlisted tests remain unsafe to skip.
