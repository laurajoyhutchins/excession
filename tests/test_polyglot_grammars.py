"""Verify explicitly prefetched grammars for every Excession syntax extension.

This is a parser-availability smoke test, not a dependency completeness test.
No network access or compiler execution occurs here.
"""
import unittest

try:
    import tree_sitter_language_pack as pack
except ImportError:
    pack = None


# Representative filenames from core/src/syntax.rs, avoiding aliases with
# duplicate parsers (e.g. .yml and .yaml).
SOURCES = (
    "source.py", "source.js", "source.ts", "source.rs", "source.go",
    "source.c", "source.cpp", "source.java", "source.kt", "source.cs",
    "source.rb", "source.php", "source.swift", "source.scala",
    "source.sh", "source.ex", "source.json", "source.yaml",
    "source.toml", "source.sql",
)


@unittest.skipIf(pack is None, "optional syntax package not installed")
class PolyglotGrammarSmoke(unittest.TestCase):
    def test_all_recognized_grammar_families_are_prefetched(self):
        registry = pack.LanguageRegistry.new()
        cached = set(pack.downloaded_languages())
        missing = []
        for filename in SOURCES:
            with self.subTest(path=filename):
                language = pack.detect_language_from_path(filename)
                self.assertTrue(language, f"no language detected for {filename}")
                if language not in cached and not registry.has_parser(language):
                    missing.append(f"{filename} ({language})")
        self.assertEqual(missing, [], "grammars missing after trusted prefetch")


if __name__ == "__main__":
    unittest.main()
