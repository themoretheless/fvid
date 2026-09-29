#!/usr/bin/env python3
"""Build a local FVid player app, without the FFmpeg-backed media feature.

This development bundle is not installed, signed with a distribution identity,
or notarized. Opening it launches the player directly, without a `play` argument.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def build(profile):
    command = [
        "cargo", "build", "--locked", "--offline", "--no-default-features",
        "--features", "player", "--bin", "fvid-player",
        "--message-format=json-render-diagnostics",
    ]
    if profile == "release":
        command.append("--release")
    print(f"Building native player ({profile})…", flush=True)
    artifact = None
    with subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, text=True) as process:
        for line in process.stdout:
            event = json.loads(line)
            if (event.get("reason") == "compiler-artifact"
                    and event.get("target", {}).get("name") == "fvid-player"
                    and event.get("executable")):
                artifact = Path(event["executable"])
        if process.wait() != 0:
            raise RuntimeError("player build failed")
    if artifact is None or not artifact.is_file():
        raise RuntimeError("Cargo did not report a player executable")
    return artifact, command


def system_linkage(binary):
    """Reject FFmpeg and other non-system dylibs instead of shipping a broken app."""
    output = subprocess.check_output(["otool", "-L", str(binary)], text=True)
    libraries = []
    for line in output.splitlines():
        if " (compatibility version " not in line:
            continue
        library = line.strip().split(" (compatibility version ", 1)[0]
        if not library.startswith(("/System/Library/", "/usr/lib/")):
            raise RuntimeError(f"player has a non-system dynamic dependency: {library}")
        libraries.append(library)
    if not libraries:
        raise RuntimeError("otool did not report Mach-O dynamic dependencies")
    return libraries


def bundle(binary, output, profile, command):
    # mkdir is exclusive, including races with another builder. Never update an
    # existing application or follow a destination symlink.
    output.mkdir(parents=True, exist_ok=False)
    try:
        contents = output / "Contents"
        executable = contents / "MacOS/FVid"
        executable.parent.mkdir(parents=True)
        resources = contents / "Resources"
        resources.mkdir()
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        libraries = system_linkage(executable)
        info = {
            "CFBundleIdentifier": "org.fvid.player",
            "CFBundleName": "FVid",
            "CFBundleDisplayName": "FVid",
            "CFBundleExecutable": "FVid",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": "0.1.0",
            "CFBundleVersion": "1",
            "NSHighResolutionCapable": True,
        }
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        changed = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
        digest = hashlib.sha256()
        with executable.open("rb") as source:
            for block in iter(lambda: source.read(1 << 20), b""):
                digest.update(block)
        provenance = {
            "revision": revision, "working_tree_dirty": bool(changed),
            "profile": profile, "build_command": command,
            "sha256": digest.hexdigest(), "dynamic_libraries": libraries,
        }
        (resources / "build.json").write_text(json.dumps(provenance, indent=2) + "\n")
        subprocess.run(["plutil", "-lint", str(contents / "Info.plist")], check=True)
    except BaseException:
        # Only the newly reserved bundle belongs to this invocation.
        shutil.rmtree(output)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New .app path; existing paths are rejected")
    parser.add_argument("--profile", choices=["dev", "release"], default="release")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("the macOS app bundle must be built on macOS")
    output = args.output.absolute()
    if output.suffix != ".app":
        parser.error("--output must end in .app")
    if os.path.lexists(output):
        parser.error("--output already exists; choose a new path")
    binary, command = build(args.profile)
    bundle(binary, output, args.profile, command)
    print(f"Built {output}; local development bundle, not installed or notarized")


if __name__ == "__main__":
    main()
