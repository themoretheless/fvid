#!/usr/bin/env python3
"""Reject FFmpeg adapters in supported native build graphs (not legacy media)."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def dependencies(manifest, features, target, offline):
    # cargo metadata includes inactive optional edges activated only through a
    # weak dependency feature. cargo tree filters these for the selected build.
    command = ["cargo", "tree", "--locked", "--prefix", "none", "--edges", "normal,build",
               "--manifest-path", str(manifest), "--target", target, *features]
    if offline:
        command.append("--offline")
    output = subprocess.check_output(command, cwd=ROOT, text=True)
    packages = {line.split()[0] for line in output.splitlines() if line.strip()}
    legacy = False
    if "fvid-media" in packages:
        feature_command = command.copy()
        feature_command[feature_command.index("--edges") + 1] = "features"
        feature_command.extend(["--invert", "fvid-media"])
        activated = subprocess.check_output(feature_command, cwd=ROOT, text=True)
        legacy = 'fvid-media feature "legacy-ffmpeg"' in activated
    return packages, legacy


def forbidden(packages):
    return sorted(package for package in packages
                  if package in {"rusty_ffmpeg", "ffmpeg"}
                  or package.startswith(("ffmpeg-", "libav")))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--target", help="Cargo target triple (defaults to rustc host)")
    parser.add_argument("--production", action="store_true",
                        help="also require production media and CUDA graphs to exclude FFmpeg")
    args = parser.parse_args()
    target = args.target
    if target is None:
        version = subprocess.check_output(["rustc", "-vV"], text=True)
        target = next(line.removeprefix("host: ") for line in version.splitlines()
                      if line.startswith("host: "))
    cases = [
        ("owned media library", ROOT / "crates/fvid-media/Cargo.toml", []),
        ("headless", ROOT / "Cargo.toml", ["--no-default-features"]),
        ("player", ROOT / "Cargo.toml", ["--no-default-features", "--features", "player"]),
        ("camera bridge", ROOT / "crates/fvid-camera-ffi/Cargo.toml", []),
    ]
    if args.production:
        cases.extend([
            ("production media", ROOT / "Cargo.toml", ["--no-default-features", "--features", "media"]),
            ("production CUDA", ROOT / "Cargo.toml", ["--no-default-features", "--features", "media-cuda"]),
            ("media library CUDA", ROOT / "crates/fvid-media/Cargo.toml", ["--no-default-features", "--features", "cuda-hw"]),
        ])
    failures = []
    for name, manifest, features in cases:
        try:
            packages, legacy = dependencies(manifest, features, target, args.offline)
        except subprocess.CalledProcessError as error:
            failures.append(f"{name}: dependency graph could not be verified (cargo exit {error.returncode})")
            continue
        found = forbidden(packages)
        if legacy:
            found.append("fvid-media/legacy-ffmpeg")
        if found:
            failures.append(f"{name}: FFmpeg dependency reached graph: {', '.join(found)}")
            continue
        print(f"{name} ({target}): {len(packages)} build packages; no FFmpeg adapter", flush=True)

    if failures:
        raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    main()
