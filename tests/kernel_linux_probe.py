#!/usr/bin/env python3
"""End-to-end kernel syntax test against a pinned upstream Git commit.

Does not build or execute the Linux kernel. This asserts only observed syntax.
"""
import argparse
import json
import subprocess
from pathlib import Path

COMMIT = "adc218676eef25575469234709c2d87185ca223a"


def run(*argv):
    return subprocess.run(argv, check=True, capture_output=True).stdout.decode().strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--binary", required=True, type=Path)
    ap.add_argument("--output", required=True, type=Path)
    args = ap.parse_args()
    repo = args.repo.resolve()
    assert run("git", "-C", str(repo), "rev-parse", "HEAD") == COMMIT

    command = [
        str(args.binary.resolve()), "kernel", "scan",
        "--repo", str(repo), "--rev", COMMIT, "--roots", "drivers/rtc"
    ]
    report = json.loads(run(*command))
    assert report["schema"] == "excession.kernel-syntax.v1"
    assert report["source_commit"] == COMMIT
    assert report["completeness"] == "CONSERVATIVE_SYNTAX_ONLY"
    assert report["source_count"] > 100
    assert report["fact_count"] > 100
    by_path = {x["source_path"]: x for x in report["sources"]}
    for path in ("drivers/rtc/Makefile", "drivers/rtc/Kconfig",
                 "drivers/rtc/rtc-cmos.c"):
        source = by_path[path]
        git_blob = run("git", "-C", str(repo), "rev-parse", f"{COMMIT}:{path}")
        assert source["source_blob"] == git_blob, path

    make = by_path["drivers/rtc/Makefile"]["facts"]
    kconfig = by_path["drivers/rtc/Kconfig"]["facts"]
    c = by_path["drivers/rtc/rtc-cmos.c"]["facts"]
    assert any(x["kind"] == "build_object" and x["target"] == "rtc-cmos.o"
               and "CONFIG_RTC_DRV_CMOS" in x["condition"] for x in make)
    assert any(x["kind"] == "kconfig_symbol" and x["subject"] == "RTC_DRV_CMOS"
               for x in kconfig)
    assert any(x["kind"] == "kconfig_select" and x["subject"] == "RTC_DRV_CMOS"
               and x["target"] == "RTC_MC146818_LIB" for x in kconfig)
    assert any(x["kind"] == "c_include" and x["target"] == "linux/module.h"
               for x in c)
    assert any(x["kind"] == "c_include" and x["target"] == "asm/i8259.h"
               and "CONFIG_X86" in x["condition"] for x in c)
    links = report["links"]
    assert any(x["kind"] == "candidate-object-source" and
               x["source_path"] == "drivers/rtc/Makefile" and
               x["target_path"] == "drivers/rtc/rtc-cmos.c" for x in links)
    assert any(x["kind"] == "candidate-config-declaration" and
               x["source_path"] == "drivers/rtc/Makefile" and
               x["target_path"] == "drivers/rtc/Kconfig" and
               "CONFIG_RTC_DRV_CMOS" in x["condition"] for x in links)
    assert any(x["kind"] == "candidate-header-include" and
               x["source_path"] == "drivers/rtc/rtc-cmos.c" and
               x["target_path"] == "include/linux/module.h" for x in links)
    # No build-configuration evaluation may be smuggled into the receipt.
    assert all(x["status"] != "VERIFIED" for src in report["sources"]
               for x in src["facts"])

    repeated = json.loads(run(*command))
    assert repeated["report_id"] == report["report_id"]
    summary = {
        k: report[k] for k in (
            "source_commit", "source_tree", "source_count", "source_bytes",
            "fact_count", "diagnostic_count", "link_count", "report_id", "completeness"
        )
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n")
    print(json.dumps(summary, sort_keys=True))


if __name__ == "__main__":
    main()
