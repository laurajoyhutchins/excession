#!/usr/bin/env python3
"""GitHub-owned request validation and pinned Excession evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

REQUEST_SCHEMA = "excession.github-analysis-request.v1"
RECEIPT_SCHEMA = "excession.github-analysis-receipt.v1"
OWNER_ID = 219002713
SHA = re.compile(r"[0-9a-f]{40}\Z")
REPO = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")
SCHEMAS = {
    "locate": "excession.agent-locate.v1",
    "context": "excession.agent-context.v1",
    "verify": "excession.agent-verification.v1",
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


def validate(value, targets):
    if not isinstance(value, dict):
        raise Refusal("request must be an object")
    kind = value.get("operation")
    permitted = {"schema", "repository", "revision", "operation", "task", "focus", "base", "limit"}
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
        if set(value) - {"schema", "repository", "revision", "operation", "task", "limit"}:
            raise Refusal("unexpected locate field")
        if not isinstance(value.get("task"), str) or not 3 <= len(value["task"]) <= 4000 or "\x00" in value["task"]:
            raise Refusal("locate task missing or invalid")
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
        argv += ["--rev", request["revision"], "--task", request["task"],
                 "--limit", str(request["limit"]), "--index-segments", "256", "--json"]
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
    parser.add_argument("mode", choices=("validate", "execute"))
    parser.add_argument("--event", type=Path)
    parser.add_argument("--actor-id", type=int, default=0)
    parser.add_argument("--targets", type=Path, default=Path("config/github-analysis-targets.json"))
    parser.add_argument("--request", type=Path)
    parser.add_argument("--binary", type=Path)
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
