"""Tests for the optional grammar-pack bridge; no external package required."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PATH = Path(__file__).resolve().parents[1] / "core/adapters/tree_sitter.py"
spec = importlib.util.spec_from_file_location("syntax_bridge", PATH)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


def span(line=0, start=0, end=8):
    return SimpleNamespace(start_line=line, start_byte=start, end_byte=end)


class FakePack:
    def __init__(self):
        self.unavailable = set()
        self.fail = False
        self.parse_calls = 0

    @staticmethod
    def detect_language_from_path(path):
        return {".rs": "rust", ".go": "go", ".py": "python", ".c": "c"}.get(
            Path(path).suffix
        )

    def has_parser(self, language):
        return language not in self.unavailable

    @staticmethod
    def ProcessConfig(**kwargs):
        assert kwargs["parse_timeout_ms"] == 1000
        assert kwargs["max_source_bytes"] == bridge.MAX_SOURCE_BYTES
        assert kwargs["diagnostics"]
        return SimpleNamespace(**kwargs)

    def process(self, content, config):
        self.parse_calls += 1
        if self.fail:
            raise RuntimeError("untrusted parser exception details")
        function = SimpleNamespace(
            kind="FUNCTION", name="answer", span=span(), children=[]
        )
        imp = SimpleNamespace(source="std", span=span(), is_wildcard=False)
        return SimpleNamespace(
            structure=[function], imports=[imp], diagnostics=[],
            metrics=SimpleNamespace(error_count=0),
        )


class ParserBridgeTests(unittest.TestCase):
    def test_multiple_languages_delegate_to_existing_parsers(self):
        fake = FakePack()
        records = bridge.parse_items([
            {"path": "src/main.rs", "content": "fn main() {}\n"},
            {"path": "src/main.go", "content": "package main\n"},
            {"path": "src/main.py", "content": "pass\n"},
        ], fake)
        self.assertEqual([r["language"] for r in records],
                         ["rust", "go", "python"])
        self.assertEqual([r["status"] for r in records], ["PARSED"] * 3)
        self.assertEqual(fake.parse_calls, 3)
        self.assertEqual(records[0]["structure"][0]["name"], "answer")
        self.assertEqual(records[0]["imports"][0]["source"], "std")

    def test_unknown_extension_is_explicitly_unsupported(self):
        fake = FakePack()
        record = bridge.parse_items([{"path": "a.opaque", "content": "anything"}], fake)[0]
        self.assertEqual(record["status"], "UNSUPPORTED_LANGUAGE")
        self.assertEqual(fake.parse_calls, 0)

    def test_missing_grammar_never_downloads(self):
        fake = FakePack()
        fake.unavailable.add("c")
        record = bridge.parse_items([{"path": "src/a.c", "content": "int f();"}], fake)[0]
        self.assertEqual(record["status"], "GRAMMAR_NOT_INSTALLED")
        self.assertEqual(record["structure"], [])
        self.assertEqual(fake.parse_calls, 0)

    def test_parser_failure_preserves_explicit_failure(self):
        fake = FakePack()
        fake.fail = True
        record = bridge.parse_items([{"path": "a.rs", "content": "fn x() {}"}], fake)[0]
        self.assertEqual(record["status"], "PARSER_ERROR")
        self.assertNotIn("untrusted", str(record))

    def test_invalid_and_large_records_refused(self):
        fake = FakePack()
        for inputs in [
            [{"path": "a.rs", "content": ""}, {"path": "a.rs", "content": ""}],
            [{"path": "a.rs", "content": "", "oid": "forged"}],
            [{"path": "a.rs", "content": "x" * (bridge.MAX_SOURCE_BYTES + 1)}],
            [{"path": "a.rs", "content": "a"}] * (bridge.MAX_FILES + 1),
        ]:
            with self.subTest(size=len(inputs)):
                with self.assertRaises(ValueError):
                    bridge.parse_items(inputs, fake)

    def test_bounded_output_never_claims_complete_extraction(self):
        fake = FakePack()
        original = bridge.MAX_OBSERVATIONS
        try:
            bridge.MAX_OBSERVATIONS = 0
            result = bridge.parse_items([{"path": "x.rs", "content": "fn x() {}"}], fake)[0]
            self.assertEqual(result["status"], "RESOURCE_LIMIT")
            self.assertEqual(result["structure"], [])
        finally:
            bridge.MAX_OBSERVATIONS = original


if __name__ == "__main__":
    unittest.main()
