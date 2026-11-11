"""Pure standard-library tests for offline grammar binary authentication."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / "tools" / "grammar_artifacts.py"
spec = importlib.util.spec_from_file_location("grammar_artifacts", PATH)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GrammarInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "rust.so").write_bytes(b"original grammar")
        self.languages = set(module.LANGUAGES)

    def test_record_and_verify(self):
        locked = module.make_inventory(self.root, "1.25.0", self.languages)
        module.verify_inventory(locked, self.root, "1.25.0", self.languages)

    def test_mutation_and_substitution_are_refused(self):
        locked = module.make_inventory(self.root, "1.25.0", self.languages)
        (self.root / "rust.so").write_bytes(b"modified grammar")
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.0", self.languages)
        (self.root / "rust.so").write_bytes(b"original grammar")
        (self.root / "new.so").write_bytes(b"unexpected")
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.0", self.languages)

    def test_missing_grammar_version_and_symlink_refused(self):
        locked = module.make_inventory(self.root, "1.25.0", self.languages)
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.1", self.languages)
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.0", set())
        (self.root / "link.so").symlink_to(self.root / "rust.so")
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.0", self.languages)

    def test_fake_digest_and_missing_library_refused(self):
        locked = module.make_inventory(self.root, "1.25.0", self.languages)
        locked["libraries"]["rust.so"] = "?" * 64
        with self.assertRaises(ValueError):
            module.verify_inventory(locked, self.root, "1.25.0", self.languages)
        (self.root / "rust.so").unlink()
        with self.assertRaises(ValueError):
            module.make_inventory(self.root, "1.25.0", self.languages)


if __name__ == "__main__":
    unittest.main()
