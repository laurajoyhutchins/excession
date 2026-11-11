"""Versioned semantic contract tests. Fixtures are synthetic, not source audits."""
import copy
import sys
from pathlib import Path
import unittest

from jsonschema import ValidationError

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from validate_semantic_contract import digest, validate_catalog, validate_report  # noqa: E402


def observation(language, specifier, path, target=None, rule="UNSUPPORTED_IMPORT_SHAPE"):
    result = {
        "kind": "IMPORTS",
        "subject": {"path": path, "blob": "a" * 40, "line": 1, "language": language},
        "specifier": specifier,
        "assessment": {
            "status": "CANDIDATE" if target else "UNRESOLVED",
            "method": "SYNTAX_RESOLUTION",
            "rule": "RELATIVE_PATH_CANDIDATE" if target else rule,
        },
        "condition_evaluation": "NOT_EVALUATED",
    }
    if target:
        result["target"] = {"path": target, "blob": "b" * 40}
    result["observation_id"] = digest(result)
    return result


def receipt():
    items = [
        observation("python", ".utils", "pkg/main.py", "pkg/utils.py"),
        observation("rust", "crate::app", "src/lib.rs", "src/app.rs"),
        observation("go", "example.org/pkg", "app/main.go", "app/pkg/util.go"),
        observation("typescript", "@app/alias", "src/main.ts", None, "BARE_OR_ALIAS_IMPORT"),
    ]
    items.sort(key=lambda x: str(x))
    payload = {
        "schema": "excession.semantic-observations.v1",
        "source_commit": "c" * 40,
        "source_tree": "d" * 40,
        "syntax_report_id": "e" * 64,
        "resolution_report_id": "f" * 64,
        "completeness": "PARTIAL_CANDIDATES_ONLY",
        "record_count": len(items),
        "gap_count": 1,
        "records": items,
        "gaps": [{"source_path": "vendor/x.c", "status": "NO_RESOLVER_FOR_LANGUAGE"}],
    }
    return {**payload, "report_id": digest(payload)}


class SemanticContractTests(unittest.TestCase):
    def test_odcs_and_json_schema_catalogues_match(self):
        validate_catalog()

    def test_four_language_candidates_and_explicit_gaps(self):
        validate_report(receipt())

    def test_wrong_observation_and_report_digests_are_rejected(self):
        item = receipt()
        item["records"][0]["specifier"] = "changed"
        with self.assertRaises(ValueError):
            validate_report(item)
        item = receipt()
        item["source_tree"] = "0" * 40
        with self.assertRaises(ValueError):
            validate_report(item)

    def test_counts_and_duplicate_ids_are_rejected(self):
        item = receipt()
        item["record_count"] += 1
        with self.assertRaises(ValueError):
            validate_report(item)
        item = receipt()
        item["records"].append(copy.deepcopy(item["records"][0]))
        item["record_count"] += 1
        body = {k: v for k, v in item.items() if k != "report_id"}
        item["report_id"] = digest(body)
        with self.assertRaises(ValueError):
            validate_report(item)

    def test_schema_refuses_claims_beyond_candidate_semantics(self):
        modifications = [
            lambda r: r.update(completeness="COMPLETE"),
            lambda r: r["records"][0]["assessment"].update(status="NATIVE_VERIFIED"),
            lambda r: r["records"][0].update(unknown_claim=True),
            lambda r: r["records"][0]["subject"].update(path="../escape.py"),
            lambda r: r["records"][0].pop("target"),
            lambda r: r["records"][3].update(target={"path": "x.ts", "blob": "a" * 40}),
            lambda r: r["records"][0].update(condition_evaluation="PROVEN_TRUE"),
            lambda r: r.update(source_commit="invalid"),
        ]
        for modify in modifications:
            item = receipt()
            modify(item)
            with self.subTest(modify=modifications.index(modify)):
                with self.assertRaises(ValidationError):
                    validate_report(item)


if __name__ == "__main__":
    unittest.main()
