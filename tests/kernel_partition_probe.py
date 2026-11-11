#!/usr/bin/env python3
"""Independent source inventory and partition audit against pinned Linux v6.12.

Small synthetic checkout proves ALL-partitions coverage, integrity refusal,
large-file unsupported accounting and repeatability. Sparse Linux checkout
proves real whole-tree inventory, plus one real C/Kbuild neighborhood slice.
This is syntax-only evidence, not configured Kbuild dependency coverage.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

LINUX = "adc218676eef25575469234709c2d87185ca223a"
ELIGIBLE = {".c", ".h"}


def command(argv, *, okay=True):
    result = subprocess.run(argv, capture_output=True, timeout=120)
    if okay and result.returncode:
        raise AssertionError(f"{argv!r}: {result.stderr.decode(errors='replace')[-1500:]}")
    return result


def cli(binary, *args):
    return json.loads(command([str(binary), "kernel", *args]).stdout)


def tracked(repo):
    output = command(["git", "-C", str(repo), "ls-tree",
                      "-r", "-t", "-z", "--full-tree", "HEAD"]).stdout
    selected = []
    for record in output.split(b"\0"):
        if not record or b"\t" not in record:
            continue
        kind, path = record.split(b"\t", 1)
        mode, entry_type, oid = kind.split(b" ")
        if entry_type != b"blob" or mode not in (b"100644", b"100755"):
            continue
        name = path.decode()
        tail = name.rsplit("/", 1)[-1]
        if (name.endswith((".c", ".h")) or
                tail in ("Makefile", "Kbuild") or tail.startswith("Kconfig")):
            selected.append((name, mode.decode(), oid.decode()))
    return sorted(selected)


def expected_digest(entries):
    h = hashlib.sha256()
    for path, mode, oid in entries:
        for field in (path, mode, oid):
            h.update(field.encode())
            h.update(b"\0")
    return h.hexdigest()


def create_fixture(root):
    repo = root / "tiny"
    repo.mkdir()
    command(["git", "init", "-q", str(repo)])
    command(["git", "-C", str(repo), "config", "user.email", "test@invalid.local"])
    command(["git", "-C", str(repo), "config", "user.name", "Fixture"])
    files = {
        "src/a.c": '#include "a.h"\nint main(void) { return 1; }\n',
        "src/a.h": "#define A 1\n",
        "src/Makefile": "obj-$(CONFIG_A) += a.o\n",
        "src/Kconfig": "config A\n bool \"Something\"\n",
        "drivers/b.c": "int other(void) { return 0; }\n",
    }
    for path, data in files.items():
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(data)
    # This oversized eligible source must be accounted for, not silently
    # discarded or claimed to be parsed.
    (repo / "src" / "huge.h").write_bytes(b"a" * (1024 * 1024 + 5))
    # 102,000 short declarations fit inside 1 MiB but exceed the bounded
    # fact budget. The file must be explicitly unsupported, not truncated.
    (repo / "src" / "budget.c").write_bytes(b"#define X\\n" * 102_000)
    (repo / "src" / "README.md").write_text("Not part of kernel source syntax\n")
    command(["git", "-C", str(repo), "add", "."])
    command(["git", "-C", str(repo), "commit", "-qm", "fixture"])
    return repo


def fixture_test(binary, root):
    repo = create_fixture(root)
    entries = tracked(repo)
    assert len(entries) == 7
    size = 2
    inv = cli(binary, "inventory", "--repo", str(repo), "--size", str(size))
    assert inv["eligible_source_count"] == len(entries)
    assert inv["partition_count"] == 4
    assert inv["inventory_sha256"] == expected_digest(entries)
    assert inv["status"] == "GIT_INVENTORY_ONLY_NOT_PARSED"
    directory = root / "receipts"
    directory.mkdir()
    for index in range(inv["partition_count"]):
        args = ("partition", "--repo", str(repo), "--size", str(size),
                "--index", str(index))
        receipt = cli(binary, *args)
        assert receipt["report_id"] == cli(binary, *args)["report_id"]
        assert receipt["partition"]["inventory_sha256"] == inv["inventory_sha256"]
        assert receipt["selected_source_count"] == len(entries[index * size:(index + 1) * size])
        (directory / f"part-{index:05}.json").write_text(json.dumps(receipt, sort_keys=True))
    audit_args = ("audit", "--repo", str(repo), "--size", str(size), "--dir", str(directory))
    audited = cli(binary, *audit_args)
    assert audited["eligible_sources"] == 7
    assert audited["observed_sources"] == 5
    assert audited["unsupported_sources"] == 2
    assert audited["unsupported_reasons"]["SOURCE_EXCEEDS_1_MIB"] == 1
    assert audited["unsupported_reasons"]["OBSERVATION_BUDGET"] == 1
    assert audited["status"] == "ALL_ELIGIBLE_SOURCES_ACCOUNTED_PARTIAL_SYNTAX"
    assert audited["report_id"] == cli(binary, *audit_args)["report_id"]
    # Omission, changed source identity and record duplication must fail.
    victim = directory / "part-00001.json"
    original = victim.read_bytes()
    victim.unlink()
    missing = command([str(binary), "kernel", *audit_args], okay=False)
    assert missing.returncode, "missing receipt accepted"
    victim.write_bytes(original)
    altered = json.loads(original)
    target = altered["sources"] or altered["unsupported"]
    target[0]["source_blob"] = "0" * 40
    victim.write_text(json.dumps(altered))
    stale = command([str(binary), "kernel", *audit_args], okay=False)
    assert stale.returncode, "changed source identity accepted"
    victim.write_bytes(original)
    duplicate = json.loads(original)
    if duplicate["sources"]:
        duplicate["sources"].append(duplicate["sources"][0])
        victim.write_text(json.dumps(duplicate))
        duplicated = command([str(binary), "kernel", *audit_args], okay=False)
        assert duplicated.returncode, "duplicate source accepted"
        victim.write_bytes(original)
    outside = command([str(binary), "kernel", "partition", "--repo", str(repo),
                       "--size", str(size), "--index", "4"], okay=False)
    assert outside.returncode
    runner = Path(__file__).with_name("linux_partition_sweep.py")
    output = root / "orchestrated"
    base = [sys.executable, str(runner), "--repo", str(repo),
            "--binary", str(binary), "--out", str(output), "--size", str(size)]
    partial = json.loads(command(base + ["--max-partitions", "1"]).stdout)
    assert partial["status"] == "INCOMPLETE_PARTITIONS_UNAUDITED"
    assert partial["completed_partitions"] == 1
    full = json.loads(command(base).stdout)
    assert full["coverage"]["report_id"] == audited["report_id"]
    assert full["status"] == "ALL_ELIGIBLE_SOURCES_ACCOUNTED_PARTIAL_SYNTAX"
    assert json.loads(command(base).stdout)["coverage"]["report_id"] == audited["report_id"]
    return {"eligible_sources": 7, "observed_sources": 5,
            "unsupported_sources": 2, "partitions": 4,
            "integrity_refusals": 4, "checkpoint_resume_verified": True}


def linux_test(binary, repo):
    assert command(["git", "-C", str(repo), "rev-parse", "HEAD"]).stdout.decode().strip() == LINUX
    entries = tracked(repo)
    assert len(entries) > 50_000, "expected kernel-wide source inventory"
    inv = cli(binary, "inventory", "--repo", str(repo), "--rev", LINUX, "--size", "32")
    assert inv["source_commit"] == LINUX
    assert inv["eligible_source_count"] == len(entries)
    assert inv["inventory_sha256"] == expected_digest(entries)
    assert inv["partition_count"] == (len(entries) + 31) // 32
    # Choose a real subsection, without relying on how many upstream files
    # appear before it. This partition may contain other driver subsystems.
    index = next(i for i, (path, _, _) in enumerate(entries)
                 if path == "drivers/rtc/rtc-cmos.c") // 32
    args = ("partition", "--repo", str(repo), "--rev", LINUX,
            "--index", str(index), "--size", "32")
    part = cli(binary, *args)
    assert part["selected_source_count"] == 32
    selected = entries[index * 32:(index + 1) * 32]
    returned = {(x["source_path"], x["source_blob"]) for x in part["sources"]}
    skipped = {(x["source_path"], x["source_blob"]) for x in part["unsupported"]}
    assert returned | skipped == {(path, oid) for path, _, oid in selected}
    assert not returned & skipped
    assert part["partition"]["inventory_sha256"] == inv["inventory_sha256"]
    assert part["report_id"] == cli(binary, *args)["report_id"]
    assert any(x["source_path"] == "drivers/rtc/rtc-cmos.c" for x in part["sources"])
    assert command(["git", "-C", str(repo), "status", "--porcelain"]).stdout == b""
    return {"commit": LINUX, "eligible_sources": len(entries),
            "inventory_sha256": inv["inventory_sha256"],
            "partition_count": inv["partition_count"],
            "sample_partition_index": index,
            "sample_observed": len(part["sources"]),
            "sample_unsupported": len(part["unsupported"]),
            "sample_report_id": part["report_id"],
            "coverage": "ONE_PARTITION_OF_WHOLE_TREE_ONLY"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--linux", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as path:
        synthetic = fixture_test(args.binary.resolve(), Path(path))
    upstream = linux_test(args.binary.resolve(), args.linux.resolve())
    report = {"schema": "excession.kernel-whole-inventory-probe.v1",
              "synthetic_full_coverage": synthetic,
              "linux_inventory_and_slice": upstream,
              "limitations": [
                  "Full Linux source tree indexed, not fully syntax parsed in this CI probe",
                  "Partition-local Kconfig symbol candidates are not a global configured build graph",
                  "No compiler or Kbuild execution was performed",
              ]}
    out.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps(report, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
