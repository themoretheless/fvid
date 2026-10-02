#!/usr/bin/env python3
"""Validate owned media builds/tests; opt in to the full external reference corpus."""
import argparse
import datetime
import json
import os
import pathlib
import re
import subprocess
import tempfile
from common import ROOT, release_binary


def save_report(destination, report):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=destination.parent, delete=False) as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
        temporary = handle.name
    os.replace(temporary, destination)


def run(command):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"{command}: exit {result.returncode}\n{result.stdout[-8192:]}\n{result.stderr[-8192:]}")
    return result


def native_validation(report):
    command = ["python3", str(ROOT / "scripts/check_native_dependencies.py"), "--offline"]
    result = run(command)
    report["checks"].append({"name": "native dependency and ordinary fixture/test policy",
                             "status": "passed", "command": command, "stdout": result.stdout})
    suites = [
        ["cargo", "test", "--manifest-path", str(ROOT / "crates/fvid-media/Cargo.toml"),
         "--no-default-features", "--lib"],
        ["cargo", "test", "--no-default-features", "--lib", "--tests"],
    ]
    for command in suites:
        result = run(command)
        summaries = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", result.stdout)
        passed = sum(int(p) for p, _, _ in summaries)
        if not passed or any(int(f) for _, f, _ in summaries):
            raise RuntimeError(f"native media suite has no successful executed tests: {command}")
        report["checks"].append({"name": "owned media Rust tests", "status": "passed",
                                 "command": command, "passed": passed,
                                 "ignored": sum(int(i) for _, _, i in summaries),
                                 "summaries": [list(summary) for summary in summaries]})
        print(f"Native media suite: {passed} tests passed", flush=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=str(release_binary()),
                        help="CLI binary used only by --benchmark-reference")
    parser.add_argument("--report")
    parser.add_argument("--benchmark-reference", action="store_true",
                        help="run the complete existing external FFmpeg/ffprobe reference corpus")
    args = parser.parse_args(argv)
    destination = pathlib.Path(args.report).resolve() if args.report else ROOT / "benchmarks" / (
        "media-validation.json" if args.benchmark_reference else "native-media-validation.json")
    report = {"created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "status": "failed", "scope": "owned media Rust tests and native dependency policy",
              "checks": [], "reference_requested": args.benchmark_reference,
              "reference_completed": False, "provided_cli_checks_completed": False}
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        if args.benchmark_reference:
            from benchmark_media_reference import main as reference_main
            report.update(reference_main(["--binary", args.binary, "--report", str(destination)]))
            report.update(reference_requested=True, reference_completed=True,
                          provided_cli_checks_completed=True, scope="full media CLI external reference corpus")
        else:
            native_validation(report)
        report["status"] = "passed"
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        save_report(destination, report)
    return report


if __name__ == "__main__":
    main()
