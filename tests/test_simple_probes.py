"""Observable simple-tool probe contracts, not agent-completion graders."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/micro"))
import locate
import symbol_compare
from simple_common import load_tasks


class SimplerToolTests(unittest.TestCase):
    def test_declared_focus_corpus(self) -> None:
        tasks = [t for t in load_tasks(ROOT / "benchmarks/calibration-h11.json")
                 if t["kind"] == "focus"]
        self.assertEqual(len(tasks), 4)
        self.assertTrue(all(len(t["expected_source_paths"]) == 1 for t in tasks))

    def test_git_file_filter(self) -> None:
        self.assertEqual(
            locate.normalize(b"h11/_connection.py\nh11/_headers.py\n",
                             "git", ROOT, "_headers.py", "pinned", "h11._headers"),
            ["h11/_headers.py"],
        )

    def test_native_output_requires_pinned_commit(self) -> None:
        for arm in ("rust_graph", "view"):
            with self.subTest(arm=arm), self.assertRaisesRegex(ValueError, "different source revision"):
                locate.normalize(json.dumps({"source_commit": "wrong", "source_paths": []}).encode(),
                                 arm, ROOT, "_headers.py", "pinned", "h11._headers")

    def test_text_occurrence_positions(self) -> None:
        source = (ROOT / symbol_compare.FILE).read_text().splitlines()
        self.assertEqual(source[0].index("compute"), 4)
        self.assertEqual(source[1].index("compute"), 24)
        stdout = ("1:" + source[0] + "\n2:" + source[1] + "\n").encode()
        self.assertEqual(symbol_compare.text_locations(stdout, "rg", "a"*40),
                         [(0, 4), (1, 24)])
        commit = "a"*40
        git = (f"{commit}:{symbol_compare.FILE}:1:{source[0]}\n"
               f"{commit}:{symbol_compare.FILE}:2:{source[1]}\n").encode()
        self.assertEqual(symbol_compare.text_locations(git, "git_grep", commit),
                         [(0, 4), (1, 24)])


if __name__ == "__main__":
    unittest.main()
