"""Optional, offline Tree-sitter language-pack bridge.

Read a JSON array of {path, content} on stdin. No checkout code is executed.
The bridge emits syntax observations, never inferred build/test dependencies.
Grammar files must already be installed; parser downloads are forbidden.
"""
from __future__ import annotations

import json
import sys

MAX_SOURCE_BYTES = 512 * 1024
MAX_OBSERVATIONS = 2048
MAX_FILES = 32


def _kind(value):
    raw = getattr(value, "value", value)
    if isinstance(raw, str):
        return raw.lower()
    return str(getattr(value, "name", raw)).lower()


def _line(span):
    return span.start_line + 1


def _structure(item, budget):
    if budget[0] <= 0:
        raise OverflowError("too many structural observations")
    budget[0] -= 1
    result = {
        "kind": _kind(item.kind),
        "name": item.name or "",
        "line": _line(item.span),
        "start_byte": item.span.start_byte,
        "end_byte": item.span.end_byte,
    }
    children = []
    for child in item.children:
        children.append(_structure(child, budget))
    result["children"] = children
    return result


def parse_items(items, pack):
    if not isinstance(items, list) or len(items) > MAX_FILES:
        raise ValueError("expected at most 32 source records")
    output = []
    seen = set()
    for item in items:
        if (not isinstance(item, dict) or set(item) != {"path", "content"}
                or not all(isinstance(v, str) for v in item.values())):
            raise ValueError("invalid source record")
        path, content = item["path"], item["content"]
        if not path or path in seen:
            raise ValueError("empty or duplicate source path")
        seen.add(path)
        data = content.encode("utf-8")
        if len(data) > MAX_SOURCE_BYTES:
            raise ValueError("source exceeds 512 KiB")
        language = pack.detect_language_from_path(path)
        result = {
            "path": path, "language": language or "",
            "status": "UNSUPPORTED_LANGUAGE", "structure": [],
            "imports": [], "diagnostics": [],
        }
        if not language:
            output.append(result)
            continue
        # has_parser() only checks locally available grammars, never downloads.
        if not pack.has_parser(language):
            result["status"] = "GRAMMAR_NOT_INSTALLED"
            output.append(result)
            continue
        try:
            config = pack.ProcessConfig(
                language=language, structure=True, imports=True,
                exports=False, symbols=False, diagnostics=True,
                max_source_bytes=MAX_SOURCE_BYTES, parse_timeout_ms=1000,
            )
            parsed = pack.process(content, config)
            budget = [MAX_OBSERVATIONS]
            structures = [_structure(item, budget) for item in parsed.structure]
            imports = []
            for imp in parsed.imports:
                if budget[0] <= 0:
                    raise OverflowError("too many import observations")
                budget[0] -= 1
                imports.append({
                    "source": imp.source,
                    "line": _line(imp.span),
                    "wildcard": bool(imp.is_wildcard),
                })
            diagnostics = []
            for diag in parsed.diagnostics:
                if budget[0] <= 0:
                    raise OverflowError("too many diagnostics")
                budget[0] -= 1
                diagnostics.append({
                    "line": _line(diag.span),
                    "message": str(diag.message)[:512],
                })
            result["structure"] = structures
            result["imports"] = imports
            result["diagnostics"] = diagnostics
            result["status"] = (
                "PARSED_WITH_ERRORS" if parsed.metrics.error_count else "PARSED"
            )
        except OverflowError:
            result["status"] = "RESOURCE_LIMIT"
            result["structure"] = []
            result["imports"] = []
            result["diagnostics"] = []
        except Exception as exc:
            # Do not expose arbitrary parser exception text as trusted evidence.
            result["status"] = "PARSER_ERROR"
            result["structure"] = []
            result["imports"] = []
            result["diagnostics"] = []
        output.append(result)
    return output


def main():
    try:
        import tree_sitter_language_pack as pack
    except ImportError as exc:
        raise SystemExit(
            "tree-sitter-language-pack is not installed; "
            "install the pinned optional parser package and prefetch grammars"
        ) from exc
    from importlib.metadata import version
    from types import SimpleNamespace
    # Module-level has_parser does not exist in v1.25. Use an offline registry
    # for both availability checks and extraction, never auto-downloading.
    registry = pack.LanguageRegistry.new()
    offline = SimpleNamespace(
        detect_language_from_path=pack.detect_language_from_path,
        has_parser=registry.has_parser,
        ProcessConfig=pack.ProcessConfig,
        process=registry.process,
    )
    items = json.load(sys.stdin)
    result = {"parser_version": version("tree-sitter-language-pack"),
              "sources": parse_items(items, offline)}
    print(json.dumps(result, sort_keys=True, separators=(",", ":"),
                     ensure_ascii=False))


if __name__ == "__main__":
    main()
