#!/usr/bin/env python3
"""Selection tests for pinned, independent product oracles (no external network)."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "benchmarks/experiments"))
from real_fixtures import select_fixtures, choose_distinct_impact_edge  # noqa: E402

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

    def test_package_self_edge_is_not_a_distinct_impact_dependent(self):
        modules = {
            "src.requests": "src/requests/__init__.py",
            "src.requests.__version__": "src/requests/__version__.py",
            "src.requests.utils": "src/requests/utils.py",
        }
        expected = {
            ("src.requests", "src.requests", 158, "from"),
            ("src.requests", "src.requests.__version__", 152, "from"),
            ("src.requests", "src.requests.utils", 158, "from"),
        }
        chosen, excluded = choose_distinct_impact_edge(expected, modules)
        self.assertEqual(chosen, ("src.requests", "src.requests.__version__", 152, "from"))
        self.assertEqual(excluded, 1)

    def test_only_package_self_edge_cannot_satisfy_impact_acceptance(self):
        with self.assertRaisesRegex(ValueError, "distinct-source"):
            choose_distinct_impact_edge(
                {("package", "package", 1, "from")},
                {"package": "src/package/__init__.py"},
            )

    def test_duplicate_manifest_names_refuse(self):
        with self.assertRaisesRegex(ValueError, "duplicate fixture names"):
            select_fixtures(FIXTURES + [{"name": "click"}], [], [])


if __name__ == "__main__":
    unittest.main()
