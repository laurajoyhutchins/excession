#!/usr/bin/env python3
"""Selection tests for pinned, independent product oracles (no external network)."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "benchmarks/experiments"))
from real_fixtures import select_fixtures  # noqa: E402

FIXTURES = [
    {"name": "h11"}, {"name": "itsdangerous"}, {"name": "click"},
    {"name": "requests"}, {"name": "axios"}, {"name": "import-cases"},
]


class HoldoutSelectionTests(unittest.TestCase):
    def test_default_preserves_existing_h11_axios_behavior(self):
        selected, products = select_fixtures(FIXTURES, [], [])
        self.assertEqual([f["name"] for f in selected],
                         [f["name"] for f in FIXTURES])
        self.assertEqual(products, frozenset(("h11", "axios")))

    def test_independent_holdouts_preserve_frozen_manifest_order(self):
        selected, products = select_fixtures(
            FIXTURES,
            ["requests", "click", "itsdangerous"],
            ["requests", "click", "itsdangerous"],
        )
        self.assertEqual([f["name"] for f in selected],
                         ["itsdangerous", "click", "requests"])
        self.assertEqual(products, frozenset(("requests", "click", "itsdangerous")))

    def test_unlisted_unknown_or_duplicate_fixtures_refuse(self):
        for selected, products in [
            (["click", "click"], ["click"]),
            (["click"], ["requests"]),
            (["missing"], []),
            (["click"], ["click", "click"]),
        ]:
            with self.subTest(selected=selected, products=products):
                with self.assertRaises(ValueError):
                    select_fixtures(FIXTURES, selected, products)

    def test_duplicate_manifest_names_refuse(self):
        with self.assertRaisesRegex(ValueError, "duplicate fixture names"):
            select_fixtures(FIXTURES + [{"name": "click"}], [], [])


if __name__ == "__main__":
    unittest.main()
