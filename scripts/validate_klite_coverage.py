#!/usr/bin/env python3
"""Run native media checks; explicitly opt in to the complete K-Lite reference benchmark."""
import argparse
import datetime
import json
import pathlib
import tempfile
from common import ROOT
from validate_media import native_validation, save_report


def reference_validation(args, report):
    # Import only after explicit opt-in: the preserved corpus inventories its
    # external oracle at import time as well as during fixture/CLI comparisons.
    from benchmark_klite_reference import main as reference_main
    with tempfile.TemporaryDirectory(prefix="fvid-klite-reference-") as folder:
        output = pathlib.Path(folder) / "coverage.json"
        argv = ["--binary", args.binary, "--out", str(output)]
        for name in ("markdown", "native_probe", "native_video_probe", "mcp_binary"):
            value = getattr(args, name)
            if value is not None:
                argv += ["--" + name.replace("_", "-"), value]
        result = reference_main(argv)
        # Read only this invocation's output, never an older published success.
        if output.exists():
            corpus = json.loads(output.read_text())
            if not isinstance(corpus, dict):
                raise RuntimeError("K-Lite reference report must be an object")
            report.update(corpus)
            report.update(reference_requested=True, reference_completed=False,
                          provided_cli_checks_completed=False,
                          scope="full K-Lite external reference corpus")
        if result != 0:
            raise RuntimeError(f"K-Lite reference benchmark failed with exit code {result}")
        rows = report.get("rows")
        if not isinstance(rows, list) or not rows or report.get("total") != len(rows):
            raise RuntimeError("K-Lite reference benchmark produced no complete row report")
    report.update(reference_completed=True, provided_cli_checks_completed=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-reference", action="store_true",
                        help="run the preserved full K-Lite corpus with external oracle tools")
    parser.add_argument("--binary", default=str(ROOT / "target-media/release/fvid"),
                        help="CLI under test, used only by --benchmark-reference")
    parser.add_argument("--out")
    parser.add_argument("--markdown", help="rewrite the measured reference matrix; requires --benchmark-reference")
    parser.add_argument("--native-probe", help="reference corpus audio probe")
    parser.add_argument("--native-video-probe", help="reference corpus video probe")
    parser.add_argument("--mcp-binary", help="reference corpus MCP binary")
    args = parser.parse_args(argv)
    if not args.benchmark_reference and any(getattr(args, name) is not None for name in
            ("markdown", "native_probe", "native_video_probe", "mcp_binary")):
        parser.error("reference matrix/probe options require --benchmark-reference")
    destination = pathlib.Path(args.out).resolve() if args.out else ROOT / "benchmarks" / (
        "klite-coverage.json" if args.benchmark_reference else "native-klite-validation.json")
    report = {"status": "failed", "checks": [],
              "scope": ("full K-Lite external reference corpus" if args.benchmark_reference
                        else "owned media Rust tests and dependency policy; no K-Lite row census"),
              "reference_requested": args.benchmark_reference, "reference_completed": False,
              "provided_cli_checks_completed": False}
    # The reference corpus promises byte-stable snapshots for identical inputs.
    # Ordinary test execution has a separate report and may record its run time.
    if not args.benchmark_reference:
        report["created_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    try:
        if args.benchmark_reference:
            reference_validation(args, report)
        else:
            native_validation(report)
        report["status"] = "passed"
    except Exception as error:
        report.update(status="failed", error=str(error), reference_completed=False,
                      provided_cli_checks_completed=False)
        raise
    finally:
        # The corpus retains every row and gate. This adapter adds completion
        # scope and atomically publishes failures as well as successes.
        save_report(destination, report)
    return report


if __name__ == "__main__":
    main()
