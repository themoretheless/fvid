#!/usr/bin/env python3
"""Prepare external K-Lite benchmark samples only after explicit reference opt-in."""
import argparse
import sys


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-reference", action="store_true",
                        help="allow reference downloads, external oracle probing and prefix generation")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--auto", action="store_true")
    parser.add_argument("--add", nargs=2, action="append", metavar=("ROW", "SOURCE"), default=[])
    args = parser.parse_args(argv)
    if not args.benchmark_reference:
        parser.error("external benchmark sample preparation requires --benchmark-reference")
    from benchmark_fetch_klite_samples import main as reference_main
    forwarded = ["--" + name for name in ("force", "verbose", "auto") if getattr(args, name)]
    for row, source in args.add:
        forwarded += ["--add", row, source]
    return reference_main(forwarded)


if __name__ == "__main__":
    sys.exit(main())
