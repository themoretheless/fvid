#!/usr/bin/env python3
"""Qualify physical CUDA tests, with opt-in external CLI reference comparisons."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
from common import ROOT, prepend_cuda_bin, release_binary
from cuda_qualification import qualify_native_tests


def save_report(destination, report):
    destination = pathlib.Path(destination).resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=destination.parent, delete=False) as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
        temporary = handle.name
    os.replace(temporary, destination)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=str(release_binary()))
    parser.add_argument("--report", default=str(ROOT / "benchmarks/hw-validation.json"))
    parser.add_argument("--benchmark-reference", action="store_true",
                        help="also execute the external FFmpeg/ffprobe CLI reference comparisons")
    args = parser.parse_args(argv)
    prepend_cuda_bin()
    binary = pathlib.Path(args.binary).resolve()
    checks = []
    report = {
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary.is_file() else None,
        "checks": checks,
        "status": "failed",
        "scope": "physical CUDA Cargo tests",
        "reference_requested": args.benchmark_reference,
        "reference_completed": False,
        "provided_cli_checks_completed": False,
    }

    def run(cmd):
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
        except OSError as error:
            report["error"] = f"{cmd}: {error}"
            raise
        if result.returncode:
            report["error"] = f"{cmd}\n{result.stdout[-4096:]}\n{result.stderr[-4096:]}"
            raise RuntimeError(f"{cmd}\n{result.stderr}")
        return result

    try:
        report["nvidia"] = run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"]).stdout.strip()
        report["native_tests"] = qualify_native_tests(ROOT, run)
        checks.append("physical CUDA point/sampling, NV12/P010, stream lifetime and NVENC tests")
        if args.benchmark_reference:
            from benchmark_hw_cuda_reference import run_reference
            run_reference(run, binary, report, checks)
            report["reference_completed"] = True
            report["provided_cli_checks_completed"] = True
            report["scope"] = "physical CUDA Cargo tests and external CLI reference comparisons"
        report["status"] = "passed"
    except Exception as error:
        report.setdefault("error", str(error))
        raise
    finally:
        save_report(args.report, report)
    return report


if __name__ == "__main__":
    main()
