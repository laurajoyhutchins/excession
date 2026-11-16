"""Markdown local links remain valid without a documentation server."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest
import sys

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("check_docs", ROOT / "tools" / "check_docs.py")
check = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = check
spec.loader.exec_module(check)


class DocumentationLinks(unittest.TestCase):
    def test_relative_source_and_anchor_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "core").mkdir()
            (root / "core" / "source.rs").write_text("fn f() {}")
            doc = root / "docs" / "guide.md"
            doc.write_text("# Guide\n[implementation](../core/source.rs#source)\n")
            self.assertEqual(check.check_document(root, doc), [])

    def test_missing_target_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            doc = root / "README.md"
            doc.write_text("See [removed](docs/removed.md).\n")
            result = check.check_document(root, doc)
            self.assertEqual(len(result), 1)
            self.assertEqual(result[0].line, 1)
            self.assertEqual(result[0].reason, "missing local target")

    def test_fenced_examples_are_not_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            doc = root / "README.md"
            fence = chr(96) * 3
            doc.write_text("# Guide\n~~~md\n[example](missing.md)\n~~~\n"
                           + fence + "md\n[example](another.md)\n" + fence + "\n")
            self.assertEqual(check.check_document(root, doc), [])

    def test_reference_links_and_images(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "diagram.svg").write_text("<svg/>")
            doc = root / "README.md"
            doc.write_text("![figure](diagram.svg)\n[readme]: <diagram.svg>\n")
            self.assertEqual(check.check_document(root, doc), [])
            doc.write_text("![figure](not-here.svg)\n")
            self.assertEqual(check.check_document(root, doc)[0].reason, "missing local target")

    def test_external_links_are_not_fetched(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            doc = root / "README.md"
            doc.write_text("[web](https://example.org/a) [mail](mailto:team@example.org)\n"
                           "[section](#local)\n")
            self.assertEqual(check.check_document(root, doc), [])

    def test_outside_repository_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "checkout"
            root.mkdir()
            doc = root / "README.md"
            doc.write_text("[outside](../external.md)\n")
            self.assertEqual(check.check_document(root, doc)[0].reason, "outside repository")

    def test_repository_absolute_link_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            doc = root / "README.md"
            doc.write_text("[bad](/docs/README.md)\n")
            self.assertEqual(check.check_document(root, doc)[0].reason, "repository-absolute link")

    def test_indexed_markdown_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "docs" / "guide.md").write_text("[missing](lost.md)")
            (root / "README.md").write_text("# Home")
            files = check.markdown_files(root)
            self.assertEqual({p.relative_to(root).as_posix() for p in files},
                             {"README.md", "docs/guide.md"})
            self.assertEqual(len(check.audit(root)), 1)


if __name__ == "__main__":
    unittest.main()
