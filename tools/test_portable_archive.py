#!/usr/bin/env python3
"""Smoke-test a release archive extracted outside the source checkout."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def run(*args: str, cwd: Path) -> str:
    output = subprocess.run(
        args, cwd=cwd, check=True, capture_output=True, text=True, encoding="utf-8"
    )
    return output.stdout


def verify(archive: Path) -> None:
    expected = (archive.parent / (archive.name + ".sha256")).read_text(encoding="ascii")
    digest, filename = expected.strip().split("  ", 1)
    assert filename == archive.name
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    with tempfile.TemporaryDirectory(prefix="excession portable archive Ω ") as folder:
        root = Path(folder)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as z:
                z.extractall(root)
        else:
            with tarfile.open(archive, "r:gz") as tar:
                tar.extractall(root, filter="data")
        installed = root / "excession"
        binary = installed / ("excession.exe" if os.name == "nt" else "excession")
        assert binary.is_file()
        assert (installed / "lib/typescript.js").is_file()
        assert (installed / "lib/TypeScript-LICENSE.txt").is_file()
        source = root / "independent Unicode repo"
        (source / "src").mkdir(parents=True)
        (source / "src/tool.js").write_text(
            "export function answer() { return 42; }\n", encoding="utf-8"
        )
        (source / "src/consumer.js").write_text(
            'import { answer } from "./tool.js";\nexport const result = answer();\n',
            encoding="utf-8",
        )
        run("git", "init", "-q", str(source), cwd=root)
        run("git", "-C", str(source), "add", "--all", cwd=root)
        run(
            "git", "-C", str(source), "-c", "user.name=Excession Portable Test",
            "-c", "user.email=portable@example.invalid",
            "commit", "-qm", "fixture", cwd=root,
        )
        assert "excession " in run(str(binary), "--version", cwd=source)
        assert "index" in run(str(binary), "--help", cwd=source)
        index_help = run(str(binary), "index", "--help", cwd=source)
        assert all(flag in index_help for flag in ("--repo", "--db", "--name"))
        import json
        doctor = json.loads(run(str(binary), "doctor", "--json", cwd=source))
        assert doctor["schema"] == "excession.capabilities.v1"
        assert doctor["sqlite"]["status"] == "BUNDLED"
        assert doctor["tools"]["git"]["status"] == "DETECTED"
        assert doctor["required_ready"] is True
        output = run(
            str(binary), "index", "--repo", str(source),
            "--db", str(root / "portable.sqlite"), "--name", "portable-js",
            "--rev", "HEAD", "--roots", "src", cwd=source,
        )
        assert "GIT_TREE_INDEXED" in output
        # A real imported dependency, changed in the second pinned Git revision.
        baseline = run("git", "-C", str(source), "rev-parse", "HEAD", cwd=root).strip()
        cache = root / "product-index.sqlite"
        analysis = json.loads(run(str(binary), "analyze", ".", "--db", str(cache),
                                  "--json", cwd=source))
        assert analysis["schema"] == "excession.analysis.v1"
        assert analysis["source_commit"] == baseline
        assert analysis["status"] == "PARTIAL"
        assert any(
            e["source"]["path"] == "src/consumer.js"
            and e["target"]["path"] == "src/tool.js"
            and all(o["assessment"] == "CANDIDATE" for o in e["evidence"])
            for e in analysis["relationships"]
        )
        (source / "src/tool.js").write_text(
            "export function answer() { return 43; }\n", encoding="utf-8"
        )
        run("git", "-C", str(source), "add", "--all", cwd=root)
        run("git", "-C", str(source), "-c", "user.name=Excession Portable Test",
            "-c", "user.email=portable@example.invalid",
            "commit", "-qm", "change utility", cwd=root)
        head = run("git", "-C", str(source), "rev-parse", "HEAD", cwd=root).strip()
        saved = root / "impact.json"
        impact = json.loads(run(str(binary), "impact", ".", "--base", baseline,
                                "--head", head, "--db", str(cache), "--json",
                                "--output", str(saved), cwd=source))
        assert json.loads(saved.read_text(encoding="utf-8")) == impact
        assert impact["schema"] == "excession.impact.v1"
        assert impact["safe_to_skip_unlisted_tests"] is False
        candidate = next(e for e in impact["effects"]
                         if e["entity"]["path"] == "src/consumer.js"
                         and e["assessment"] == "CANDIDATE_PATH")
        claim_id = candidate["claim_id"]
        explanation = json.loads(run(str(binary), "explain", ".", "--report", str(saved),
                                     "--id", claim_id, "--db", str(cache),
                                     "--json", cwd=source))
        assert explanation["schema"] == "excession.explanation.v1"
        assert explanation["claim_id"] == claim_id
        assert explanation["steps"]
        audit = json.loads(run(str(binary), "explain", ".", "--report", str(saved),
                               "--id", claim_id, "--db", str(cache),
                               "--audit", cwd=source))
        assert audit["git_authentication"] == "PASS"
        assert audit["replay"] == "PASS"
        assert audit["semantic_correctness"] == "NOT_EVALUATED"
        impact["effects"][0]["assessment"] = "OBSERVED_RELATIONSHIP_PATH"
        saved.write_text(json.dumps(impact, sort_keys=True) + "\n", encoding="utf-8")
        tampered = subprocess.run(
            [str(binary), "explain", ".", "--report", str(saved), "--id", claim_id,
             "--db", str(cache), "--json"],
            cwd=source, capture_output=True, text=True, encoding="utf-8"
        )
        assert tampered.returncode != 0, "tampered claim was accepted"



def main() -> None:
    folder = Path(sys.argv[1])
    archives = list(folder.glob("*.zip")) + list(folder.glob("*.tar.gz"))
    if len(archives) != 1:
        raise SystemExit(f"expected one portable archive, found {len(archives)}")
    verify(archives[0])
    print(f"Verified {archives[0].name}: checksums, executable, JS parser, Git, bundled SQLite and doctor")


if __name__ == "__main__":
    main()
