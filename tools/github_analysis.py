#!/usr/bin/env python3
"""GitHub-owned request validation and pinned Excession evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys

REQUEST_SCHEMA = "excession.github-analysis-request.v1"
RECEIPT_SCHEMA = "excession.github-analysis-receipt.v1"
OWNER_ID = 219002713
SHA = re.compile(r"[0-9a-f]{40}\Z")
REPO = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
SCHEMAS = {
    "locate": "excession.agent-locate.v1",
    "context": "excession.agent-context.v1",
    "verify": "excession.agent-verification.v1",
}
# Trusted defaults match the native CLI. A caller can override work budgets,
# but not the workflow's separate OS/CI execution safety envelope.
LOCATE_INDEX_DEFAULTS = {
    "max_file_bytes": 1024 * 1024,
    "max_segment_bytes": 32 * 1024 * 1024,
    "max_segment_files": 6000,
    "refresh_segments": 256,
}



class Refusal(ValueError):
    pass


def no_duplicates(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise Refusal("duplicate JSON key: " + key)
        value[key] = item
    return value


def load_request(text):
    if len(text.encode("utf-8")) > 8192:
        raise Refusal("request exceeds 8 KiB")
    text = text.strip()
    fence = chr(96) * 3
    if text.startswith(fence + "json\n") and text.endswith("\n" + fence):
        text = text[len(fence) + 5:-len(fence) - 1].strip()
    try:
        return json.loads(text, object_pairs_hook=no_duplicates)
    except (TypeError, ValueError) as exc:
        raise Refusal("invalid JSON request") from exc


def load_targets(path):
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=no_duplicates)
    if not isinstance(value, dict) or set(value) != {"schema", "repositories"} or value["schema"] != "excession.github-analysis-targets.v1":
        raise Refusal("invalid trusted allowlist")
    repositories = value["repositories"]
    if not isinstance(repositories, list) or not 1 <= len(repositories) <= 24 or any(
        not isinstance(name, str) or not REPO.fullmatch(name) for name in repositories
    ) or len(set(repositories)) != len(repositories):
        raise Refusal("invalid allowlist entries")
    return set(repositories)


def valid_focus(value):
    return isinstance(value, list) and 1 <= len(value) <= 16 and all(
        isinstance(path, str) and 1 <= len(path) <= 512
        and not any(c in path for c in (",", "\\", "\x00"))
        and not path.startswith("/")
        and all(part not in ("", ".", "..") for part in path.split("/"))
        for path in value
    )


def validate_budgets(raw):
    if not isinstance(raw, dict) or not raw or set(raw) - set(LOCATE_INDEX_DEFAULTS):
        raise Refusal("invalid locate budgets")
    for key, value in raw.items():
        if type(value) is not int or value < (256 if key in ("max_file_bytes", "max_segment_bytes") else 1) or value > (2**63 - 1):
            raise Refusal("locate budgets must be positive integers in SQLite range")
        if key == "refresh_segments" and value > 256:
            raise Refusal("refresh_segments must be in 1..256")
    limits = {**LOCATE_INDEX_DEFAULTS, **raw}
    if limits["max_file_bytes"] > limits["max_segment_bytes"]:
        raise Refusal("max_file_bytes must not exceed max_segment_bytes")
    return dict(sorted(raw.items()))


def validate(value, targets):
    if not isinstance(value, dict):
        raise Refusal("request must be an object")
    kind = value.get("operation")
    permitted = {"schema", "repository", "revision", "operation", "task", "focus", "base", "limit", "budgets", "enrich", "resume_run_id"}
    if set(value) - permitted or kind not in SCHEMAS:
        raise Refusal("unknown request field or operation")
    if value.get("schema") != REQUEST_SCHEMA:
        raise Refusal("wrong request schema")
    if value.get("repository") not in targets:
        raise Refusal("repository not allowlisted")
    if not isinstance(value.get("revision"), str) or not SHA.fullmatch(value["revision"]):
        raise Refusal("revision must be an exact lowercase 40-character SHA")
    limit = value.get("limit", 16)
    if type(limit) is not int or not 1 <= limit <= 64:
        raise Refusal("limit outside 1..64")
    if kind == "locate":
        if set(value) - {"schema", "repository", "revision", "operation", "task", "limit", "budgets", "enrich", "resume_run_id"}:
            raise Refusal("unexpected locate field")
        if not isinstance(value.get("task"), str) or not 3 <= len(value["task"]) <= 4000 or "\x00" in value["task"]:
            raise Refusal("locate task missing or invalid")
        if "budgets" in value:
            validate_budgets(value["budgets"])
        if "enrich" in value and type(value["enrich"]) is not bool:
            raise Refusal("locate enrich must be a boolean")
        if "resume_run_id" in value:
            if (type(value["resume_run_id"]) is not int
                    or not 1 <= value["resume_run_id"] <= 2**63 - 1
                    or value.get("enrich") is not True):
                raise Refusal("locate resume_run_id requires positive integer and enrich=true")
    elif kind == "context":
        if set(value) - {"schema", "repository", "revision", "operation", "focus", "task", "limit"}:
            raise Refusal("unexpected context field")
        if not valid_focus(value.get("focus")):
            raise Refusal("invalid focus paths")
        task = value.get("task")
        if task is not None and (not isinstance(task, str) or not 3 <= len(task) <= 4000 or "\x00" in task):
            raise Refusal("invalid context task")
    else:
        if set(value) - {"schema", "repository", "revision", "operation", "base", "limit"}:
            raise Refusal("unexpected verification field")
        if not isinstance(value.get("base"), str) or not SHA.fullmatch(value["base"]):
            raise Refusal("verification base must be an exact commit")
    return {**value, "limit": limit}


def cache_identity(request, tool_commit):
    """Deterministic source/tool/budget identity for untrusted *advisory* index caches.

    Deliberately exclude the natural-language question. All index rows are
    still revalidated against the current Git tree by the native locator.
    """
    if request["operation"] != "locate":
        return None
    if not SHA.fullmatch(tool_commit):
        raise Refusal("invalid trusted tool commit for cache identity")
    budgets = {**LOCATE_INDEX_DEFAULTS, **request.get("budgets", {})}
    identity = {
        "cache_protocol": "excession.github-locate-cache.v1",
        "producer_version": "agent-locate-segments-v3",
        "tool_commit": tool_commit,
        "repository": request["repository"],
        "revision": request["revision"],
        "budgets": budgets,
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


CHECKPOINT_SCHEMA = "excession.github-locate-checkpoint.v1"
CHECKPOINT_MAX_BYTES = 512 * 1024 * 1024


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while True:
            block = source.read(1024 * 1024)
            if not block:
                break
            digest.update(block)
    return digest.hexdigest()


def checkpoint_report(request, report_directory, tool_commit):
    if request["operation"] != "locate":
        return None
    report_file = report_directory / "report.json"
    receipt_file = report_directory / "receipt.json"
    receipt = json.loads(receipt_file.read_text(encoding="utf-8"))
    report = json.loads(report_file.read_text(encoding="utf-8"))
    if (receipt.get("status") != "COMPLETED"
            or receipt.get("tool_commit") != tool_commit
            or receipt.get("request") != request
            or receipt.get("report_sha256") != file_digest(report_file)
            or receipt.get("report_id") != report.get("report_id")):
        raise Refusal("checkpoint source report/receipt identity mismatch")
    check_report(request, report, receipt.get("source_tree"))
    if report.get("index") is None:
        return None
    if not SHA.fullmatch(receipt["source_tree"]):
        raise Refusal("checkpoint source tree is not an exact Git identity")
    return report, receipt


def publish_checkpoint(request, report_directory, destination, tool_commit, run_id, cache_root):
    """Save an advisory, source-verified locator snapshot as an immutable artifact.

    An issue-originated worker cannot publish a writable Actions cache. This
    explicit artifact may be reused only with a matching tool/source/budget
    identity and the next worker reauthenticates the Git blobs.
    """
    evidence = checkpoint_report(request, report_directory, tool_commit)
    if evidence is None:
        return False
    report, receipt = evidence
    if type(run_id) is not int or run_id <= 0:
        raise Refusal("checkpoint requires a valid GitHub workflow run ID")
    source = Path(report["cache"]["path"])
    allowed_root = cache_root.resolve(strict=True)
    if (source.is_symlink() or not source.is_file()
            or not source.resolve(strict=True).is_relative_to(allowed_root)
            or source.name != "agent-locate-chunks.sqlite"):
        raise Refusal("checkpoint is not a native locator SQLite file")
    if source.stat().st_size > CHECKPOINT_MAX_BYTES:
        raise Refusal("checkpoint exceeds explicit 512 MiB artifact limit")
    destination.mkdir(parents=True, exist_ok=False)
    archive = destination / "locator.sqlite"
    try:
        with sqlite3.connect(f"file:{source.as_posix()}?mode=ro", uri=True) as current:
            with sqlite3.connect(str(archive)) as stable:
                current.backup(stable)
        if archive.stat().st_size > CHECKPOINT_MAX_BYTES:
            raise Refusal("checkpoint exceeds explicit 512 MiB artifact limit")
        manifest = {
            "schema": CHECKPOINT_SCHEMA,
            "cache_identity": cache_identity(request, tool_commit),
            "run_id": run_id,
            "source_repository": request["repository"],
            "source_commit": request["revision"],
            "source_tree": receipt["source_tree"],
            "tool_commit": tool_commit,
            "report_sha256": receipt["report_sha256"],
            "sqlite_sha256": file_digest(archive),
            "sqlite_bytes": archive.stat().st_size,
            "ready_segments": report["index"]["ready_segments"],
            "pending_segments": report["index"]["pending_segments"],
        }
        (destination / "manifest.json").write_text(
            json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
        )
    except Exception:
        shutil.rmtree(destination)
        raise
    return True


def restore_checkpoint(request, archive_root, run_metadata, tool_commit, cache_root, report_root):
    """Reject incompatible or untrusted artifacts before native Git validation."""
    if request["operation"] != "locate" or "resume_run_id" not in request:
        raise Refusal("resume requires a pinned locate request")
    manifest_path = archive_root / "manifest.json"
    sqlite_path = archive_root / "locator.sqlite"
    if (manifest_path.is_symlink() or sqlite_path.is_symlink()
            or not manifest_path.is_file() or not sqlite_path.is_file()):
        raise Refusal("checkpoint artifact missing or contains symlink")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    expected_keys = {
        "schema", "cache_identity", "run_id", "source_repository",
        "source_commit", "source_tree", "tool_commit", "report_sha256",
        "sqlite_sha256", "sqlite_bytes", "ready_segments", "pending_segments",
    }
    if not isinstance(manifest, dict) or set(manifest) != expected_keys:
        raise Refusal("checkpoint manifest schema has unknown/missing properties")
    if (manifest["schema"] != CHECKPOINT_SCHEMA
            or manifest["cache_identity"] != cache_identity(request, tool_commit)
            or manifest["run_id"] != request["resume_run_id"]
            or manifest["source_repository"] != request["repository"]
            or manifest["source_commit"] != request["revision"]
            or manifest["tool_commit"] != tool_commit
            or not isinstance(manifest["source_tree"], str)
            or not SHA.fullmatch(manifest["source_tree"])
            or not isinstance(manifest["report_sha256"], str)
            or not SHA256.fullmatch(manifest["report_sha256"])
            or not isinstance(manifest["sqlite_sha256"], str)
            or not SHA256.fullmatch(manifest["sqlite_sha256"])
            or type(manifest["sqlite_bytes"]) is not int
            or not 0 < manifest["sqlite_bytes"] <= CHECKPOINT_MAX_BYTES
            or type(manifest["ready_segments"]) is not int
            or type(manifest["pending_segments"]) is not int):
        raise Refusal("checkpoint identity or manifest value mismatch")
    if (not isinstance(run_metadata, dict)
            or run_metadata.get("id") != request["resume_run_id"]
            or run_metadata.get("event") != "issues"
            or run_metadata.get("conclusion") != "success"
            or run_metadata.get("name") != "Excession agent analysis (GitHub-native)"
            or run_metadata.get("head_sha") != tool_commit
            or run_metadata.get("head_branch") != "main"
            or run_metadata.get("actor", {}).get("id") != OWNER_ID
            or run_metadata.get("repository", {}).get("full_name") != "laurajoyhutchins/excession"):
        raise Refusal("checkpoint must come from successful owner-authorized main workflow")
    if (sqlite_path.stat().st_size != manifest["sqlite_bytes"]
            or file_digest(sqlite_path) != manifest["sqlite_sha256"]):
        raise Refusal("checkpoint SQLite checksum or length mismatch")
    with sqlite3.connect(f"file:{sqlite_path.as_posix()}?mode=ro", uri=True) as db:
        if db.execute("PRAGMA quick_check").fetchone() != ("ok",):
            raise Refusal("checkpoint SQLite integrity check failed")
        version = db.execute(
            "SELECT value FROM locator_meta WHERE key='version'"
        ).fetchone()
        if version != ("agent-locate-segments-v3",):
            raise Refusal("checkpoint native index schema differs from expected")
    # The worker clones this exact path during 'execute'. The native
    # discovery::cache_path hashes the *absolute source root*, not a branch.
    source_root = (report_root / "work" / "source").resolve()
    cache_id = hashlib.sha256(str(source_root).encode("utf-8")).hexdigest()[:24]
    cache_root.mkdir(parents=True, exist_ok=True)
    destination_dir = cache_root.resolve() / cache_id
    destination_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    destination = destination_dir / "agent-locate-chunks.sqlite"
    if destination.is_symlink():
        raise Refusal("checkpoint cannot overwrite a symlink")
    temporary = destination_dir / "locator-restoring.sqlite"
    shutil.copyfile(sqlite_path, temporary)
    temporary.replace(destination)
    return destination


def from_event(event, actor_id, targets):
    if event.get("action") == "opened" and isinstance(event.get("issue"), dict):
        issue = event["issue"]
        if issue.get("title") != "Excession analysis request" or "pull_request" in issue:
            raise Refusal("not a request issue")
        if not isinstance(issue.get("user"), dict) or issue["user"].get("id") != OWNER_ID:
            raise Refusal("only owner-authored issues are admitted")
        raw = issue.get("body")
    elif isinstance(event.get("inputs"), dict):
        if actor_id != OWNER_ID:
            raise Refusal("only owner-dispatched workflows are admitted")
        raw = event["inputs"].get("request")
    else:
        raise Refusal("unrecognized event")
    if not isinstance(raw, str):
        raise Refusal("request body missing")
    return validate(load_request(raw), targets)


def git(*argv, cwd=None, timeout=90):
    process = subprocess.run(["git", *argv], cwd=cwd, capture_output=True, text=True,
                             timeout=timeout, check=False,
                             env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
    if process.returncode:
        raise Refusal("Git object unavailable or failed verification")
    return process.stdout.strip()


def prepare_source(request, directory):
    directory.mkdir(parents=True, exist_ok=True)
    source = directory / "source"
    git("init", "--quiet", str(source))
    git("remote", "add", "origin", "https://github.com/" + request["repository"] + ".git", cwd=source)
    for sha in (request["revision"], request.get("base")):
        if sha is None:
            continue
        git("-c", "protocol.version=2", "fetch", "--quiet", "--no-tags",
            "--depth=1", "origin", sha, cwd=source, timeout=180)
        if git("rev-parse", "--verify", sha + "^{commit}", cwd=source) != sha:
            raise Refusal("fetched commit identity mismatch")
    # Excession's Git discovery expects a resolvable HEAD. Point it at the
    # exact requested source without checking out or executing source files.
    git("update-ref", "refs/heads/excession-analysis", request["revision"], cwd=source)
    git("symbolic-ref", "HEAD", "refs/heads/excession-analysis", cwd=source)
    tree = git("rev-parse", request["revision"] + "^{tree}", cwd=source)
    return source, tree


def prepare_checkout_source(request, checkout):
    """Use only Git objects already fetched into the private caller repository.

    No network fetch, checkout of the candidate, source execution, or ref
    mutation occurs here. The caller workflow supplies the private checkout.
    """
    checkout = checkout.resolve(strict=True)
    if Path(git("rev-parse", "--show-toplevel", cwd=checkout)).resolve() != checkout:
        raise Refusal("private source must be the root of a Git checkout")
    for sha in (request["revision"], request.get("base")):
        if sha is not None and git(
            "rev-parse", "--verify", sha + "^{commit}", cwd=checkout
        ) != sha:
            raise Refusal("requested commit unavailable in private checkout")
    tree = git("rev-parse", request["revision"] + "^{tree}", cwd=checkout)
    if not SHA.fullmatch(tree):
        raise Refusal("private checkout tree identity invalid")
    return checkout, tree


def command(request, source, binary):
    kind = request["operation"]
    argv = [str(binary), "agent", kind, "--repo", str(source)]
    if kind == "locate":
        budgets = {**LOCATE_INDEX_DEFAULTS, **request.get("budgets", {})}
        argv += ["--rev", request["revision"], "--task", request["task"],
                 "--limit", str(request["limit"]),
                 "--index-segments", str(budgets["refresh_segments"]),
                 "--max-file-bytes", str(budgets["max_file_bytes"]),
                 "--max-segment-bytes", str(budgets["max_segment_bytes"]),
                 "--max-segment-files", str(budgets["max_segment_files"]), "--json"]
        if request.get("enrich", False):
            argv.append("--enrich")
    elif kind == "context":
        argv += ["--rev", request["revision"], "--focus", ",".join(request["focus"]),
                 "--limit", str(request["limit"])]
        if "task" in request:
            argv += ["--task", request["task"]]
    else:
        argv += ["--base", request["base"], "--candidate", request["revision"],
                 "--limit", str(request["limit"])]
    return argv


def check_report(request, report, tree):
    if not isinstance(report, dict) or report.get("schema") != SCHEMAS[request["operation"]]:
        raise Refusal("unsupported native report schema")
    if request["operation"] == "verify":
        if (report.get("candidate_commit") != request["revision"]
                or report.get("candidate_tree") != tree
                or report.get("base_commit") != request["base"]):
            raise Refusal("verification identities disagree with Git")
    elif report.get("source_commit") != request["revision"] or report.get("source_tree") != tree:
        raise Refusal("source identities disagree with Git")
    if request["operation"] == "locate":
        # Git-only discovery legitimately omits FTS evidence. Accept *only*
        # the native explicit fast-path statement, never a missing index
        # masquerading as completed enrichment.
        index = report.get("index")
        cache = report.get("cache")
        if index is None:
            if (request.get("enrich", False)
                    or report.get("enrichment") != "NOT_REQUESTED_GIT_SUFFICIENT"
                    or report.get("retrieval") != "COMMITTED_GIT_GREP"
                    or cache is not None):
                raise Refusal("native locate report omitted required index evidence")
            return
        if (not isinstance(index, dict)
                or not isinstance(cache, dict)
                or report.get("enrichment") != "PERFORMED"):
            raise Refusal("native locate index/cache evidence invalid")
        expected = {**LOCATE_INDEX_DEFAULTS, **request.get("budgets", {})}
        observed = index.get("limits", {})
        for key, field in (
            ("max_file_bytes", "max_file_bytes"),
            ("max_segment_bytes", "max_bytes_per_segment"),
            ("max_segment_files", "max_files_per_segment"),
        ):
            if observed.get(field) != expected[key]:
                raise Refusal("native report indexing budgets differ from request")
        if cache.get("refresh_budget_segments") != expected["refresh_segments"]:
            raise Refusal("native report refresh budget differs from request")


def execute(request, binary, output, source_checkout=None, tool_checkout=None):
    output.mkdir(parents=True, exist_ok=True)
    tool_commit = git("rev-parse", "HEAD", cwd=tool_checkout)
    receipt = {
        "schema": RECEIPT_SCHEMA, "status": "REFUSED", "request": request,
        "tool_commit": tool_commit, "workflow_run_id": os.environ.get("GITHUB_RUN_ID"),
        "safe_to_skip_unlisted_tests": False,
    }
    try:
        if not binary.is_file():
            raise Refusal("Excession executable missing")
        version = subprocess.run([str(binary), "--version"], capture_output=True, text=True,
                                 timeout=10, check=True).stdout.strip()
        receipt["tool_version"] = version
        if source_checkout is None:
            source, tree = prepare_source(request, output / "work")
        else:
            source, tree = prepare_checkout_source(request, source_checkout)
        receipt["source_tree"] = tree
        report_file = output / "report.json"
        with report_file.open("wb") as sink:
            run = subprocess.run(command(request, source, binary), stdout=sink,
                                 stderr=subprocess.PIPE, timeout=240, check=False)
        if report_file.stat().st_size > 8 * 1024 * 1024:
            raise Refusal("report exceeded 8 MiB")
        if run.returncode:
            receipt["producer_exit_code"] = run.returncode
            # Public-source diagnostics only; no environment dump or tokens.
            receipt["producer_stderr_excerpt"] = run.stderr.decode("utf-8", "replace")[:1536]
            raise Refusal("native Excession CLI refused execution")
        raw = report_file.read_bytes()
        report = json.loads(raw.decode("utf-8"))
        check_report(request, report, tree)
        receipt.update(status="COMPLETED", report_schema=report["schema"],
                       report_sha256=hashlib.sha256(raw).hexdigest(),
                       report_id=report.get("report_id"))
    except (Refusal, subprocess.SubprocessError, OSError, ValueError, UnicodeError) as exc:
        receipt["reason"] = str(exc) if isinstance(exc, Refusal) else type(exc).__name__
        (output / "report.json").unlink(missing_ok=True)
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    summary = [
        "### Excession source analysis",
        "",
        "- Status: **" + receipt["status"] + "**",
        "- Repository: " + request["repository"],
        "- Commit: " + request["revision"],
        "- Operation: " + request["operation"],
        "- Tool commit: " + tool_commit,
        "- Advisory evidence only. Unlisted tests are never safe to skip.",
    ]
    if receipt["status"] != "COMPLETED":
        summary.append("- Refusal: " + receipt["reason"])
    (output / "summary.md").write_text("\n".join(summary) + "\n")
    if receipt["status"] != "COMPLETED":
        raise Refusal(receipt["reason"])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate", "execute", "cache-identity", "publish-checkpoint", "restore-checkpoint"))
    parser.add_argument("--event", type=Path)
    parser.add_argument("--actor-id", type=int, default=0)
    parser.add_argument("--targets", type=Path, default=Path("config/github-analysis-targets.json"))
    parser.add_argument("--request", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--report-dir", type=Path)
    parser.add_argument("--checkpoint-dir", type=Path)
    parser.add_argument("--run-metadata", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--trusted-repository")
    parser.add_argument("--local-checkout", type=Path)
    parser.add_argument("--tool-checkout", type=Path)
    args = parser.parse_args()
    try:
        if args.trusted_repository is not None:
            if not REPO.fullmatch(args.trusted_repository):
                raise Refusal("invalid trusted caller repository")
            if os.environ.get("GITHUB_REPOSITORY") != args.trusted_repository:
                raise Refusal("trusted repository must match the GitHub caller")
            targets = {args.trusted_repository}
        else:
            targets = load_targets(args.targets)
        if args.local_checkout is not None and (
            args.trusted_repository is None or args.tool_checkout is None
        ):
            raise Refusal("private checkout requires a trusted caller and tool checkout")
        if args.mode == "validate":
            if args.event is None:
                raise Refusal("event path required")
            request = from_event(json.loads(args.event.read_text(encoding="utf-8")),
                                 args.actor_id, targets)
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(request, sort_keys=True) + "\n")
        elif args.mode == "cache-identity":
            if args.request is None:
                raise Refusal("cache identity request missing")
            request = validate(json.loads(args.request.read_text(encoding="utf-8")), targets)
            identity = cache_identity(request, git("rev-parse", "HEAD"))
            args.out.parent.mkdir(parents=True, exist_ok=True)
            with args.out.open("a", encoding="utf-8") as output:
                output.write("prefix=" + (identity or "") + "\n")
                output.write("resume_run_id=" + str(request.get("resume_run_id", "")) + "\n")
        elif args.mode == "publish-checkpoint":
            if args.request is None or args.report_dir is None:
                raise Refusal("checkpoint publishing needs source report and request")
            request = validate(json.loads(args.request.read_text(encoding="utf-8")), targets)
            root = Path(os.environ["EXCESSION_CACHE_DIR"])
            published = publish_checkpoint(
                request, args.report_dir, args.out, git("rev-parse", "HEAD"),
                int(os.environ["GITHUB_RUN_ID"]), root,
            )
            print("checkpoint artifact: " + ("PUBLISHED" if published else "NO_INDEX"))
        elif args.mode == "restore-checkpoint":
            if args.request is None or args.checkpoint_dir is None or args.run_metadata is None:
                raise Refusal("checkpoint restoration needs manifest, request and trusted run data")
            request = validate(json.loads(args.request.read_text(encoding="utf-8")), targets)
            restored = restore_checkpoint(
                request, args.checkpoint_dir,
                json.loads(args.run_metadata.read_text(encoding="utf-8")),
                git("rev-parse", "HEAD"), Path(os.environ["EXCESSION_CACHE_DIR"]),
                Path(os.environ["RUNNER_TEMP"]) / "excession-evidence",
            )
            args.out.write_text(str(restored) + "\n", encoding="utf-8")
        else:
            if args.request is None or args.binary is None:
                raise Refusal("request and binary required")
            request = validate(json.loads(args.request.read_text(encoding="utf-8")), targets)
            execute(request, args.binary.resolve(), args.out.resolve(),
                    source_checkout=args.local_checkout,
                    tool_checkout=args.tool_checkout)
        return 0
    except (Refusal, OSError, subprocess.SubprocessError, ValueError) as exc:
        print("EXCESSION_ANALYSIS_REFUSED: " + str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
