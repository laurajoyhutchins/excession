"""Strict local-only contract loader for product scaffold tests.

This is a test convenience, never an authority for Git replay or semantic truth.
"""
from __future__ import annotations

import json
from pathlib import Path
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "https://raw.githubusercontent.com/laurajoyhutchins/excession/main/contracts/product/"


def _retrieve(uri: str) -> Resource:
    if not uri.startswith(PREFIX):
        raise ValueError(f"external schema retrieval prohibited: {uri}")
    relative = uri[len(PREFIX):]
    if not relative or "/" in relative or ".." in relative or not relative.endswith(".schema.json"):
        raise ValueError(f"invalid product schema URL: {uri}")
    return Resource.from_contents(json.loads((ROOT / "contracts" / "product" / relative).read_text()))


def validate_schema(name: str, document: dict) -> None:
    schema = json.loads((ROOT / "contracts" / "product" / name).read_text())
    Draft202012Validator.check_schema(schema)
    Draft202012Validator(schema, registry=Registry(retrieve=_retrieve)).validate(document)
