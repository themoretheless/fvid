#!/usr/bin/env python3
"""Reject FFmpeg adapters in production and native build graphs."""
import argparse
import ast
from pathlib import Path
import subprocess
import re

ROOT = Path(__file__).resolve().parents[1]


def library_uses_ffmpeg(root):
    """The reference marker must never resurrect library backend/linkage."""
    source = (root / "crates/fvid-media/src/lib.rs").read_text()
    build = (root / "crates/fvid-media/build.rs").read_text()
    return bool(re.search(r'include!\s*\(\s*"(?:legacy|av)\.rs"', source)
                or re.search(r'libav(?:codec|format|filter|util|device|resample)/', build)
                or re.search(r'cargo:rustc-link-lib[^\n]*(?:avcodec|avformat|avfilter|avutil|avdevice|swresample|swscale)', build))


def dependencies(manifest, features, target, offline):
    # cargo metadata includes inactive optional edges activated only through a
    # weak dependency feature. cargo tree filters these for the selected build.
    command = ["cargo", "tree", "--locked", "--prefix", "none", "--edges", "normal,build",
               "--manifest-path", str(manifest), "--target", target, *features]
    if offline:
        command.append("--offline")
    output = subprocess.check_output(command, cwd=ROOT, text=True)
    packages = {line.split()[0] for line in output.splitlines() if line.strip()}
    legacy = "fvid-media" in packages and library_uses_ffmpeg(ROOT)
    return packages, legacy


def forbidden(packages):
    return sorted(package for package in packages
                  if package in {"rusty_ffmpeg", "ffmpeg"}
                  or package.startswith(("ffmpeg-", "libav")))


def external_test_calls(source):
    """Detect known external reference hooks and literal tool launches.

    This source gate complements the dependency graph audit; it does not claim
    to resolve arbitrary computed subprocess paths. Saved oracle bytes and
    comments describing their provenance remain valid ordinary test inputs.
    """
    patterns = [
        r'\b(?:var_os|var|option_env!|env!)\s*\(\s*"[^"]*(?:FFMPEG|FFPROBE)[^"]*"',
        r'\bCommand\s*::\s*new\s*\(\s*"(?:[^"\n]*/)?(?:ffmpeg|ffprobe)(?:\.exe)?"',
    ]
    return sorted({source.count("\n", 0, match.start()) + 1
                   for pattern in patterns for match in re.finditer(pattern, source)})


def audit_ordinary_tests(root):
    failures = []
    paths = sorted(set((root / "tests").rglob("*.rs"))
                   | set((root / "src").rglob("*.rs"))
                   | set((root / "crates").glob("*/src/**/*.rs"))
                   | set((root / "crates").glob("*/tests/**/*.rs")))
    for path in paths:
        for line in external_test_calls(path.read_text()):
            failures.append(f"{path.relative_to(root)}:{line}: external FFmpeg test hook; move reference execution to an explicit benchmark")
    return paths, failures


def external_python_calls(source):
    """Known literal launches/environment hooks, not computed-path resolution.

    Parse code rather than comments, docstrings or saved-oracle filenames.
    """
    tree = ast.parse(source)
    lines = set()
    launches = {"run", "call", "check_call", "check_output", "Popen", "execute", "invoke", "success"}
    def literal(node):
        return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else None
    def tool(value):
        return value is not None and re.fullmatch(r"(?:[^\n]*/)?(?:ffmpeg|ffprobe)(?:\.exe)?", value, re.I)
    def environment(value):
        return value is not None and any(name in value.upper() for name in ("FFMPEG", "FFPROBE"))
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and node.args:
            name = node.func.attr if isinstance(node.func, ast.Attribute) else getattr(node.func, "id", "")
            first = node.args[0]
            command = first.elts[0] if isinstance(first, (ast.List, ast.Tuple)) and first.elts else first
            if name in launches and tool(literal(command)):
                lines.add(node.lineno)
            if name in {"getenv", "var", "var_os"} and environment(literal(first)):
                lines.add(node.lineno)
            if isinstance(node.func, ast.Attribute) and node.func.attr == "get" and isinstance(node.func.value, ast.Attribute) and node.func.value.attr == "environ" and environment(literal(first)):
                lines.add(node.lineno)
            if name == "which" and tool(literal(first)):
                lines.add(node.lineno)
        if isinstance(node, ast.Subscript) and isinstance(node.value, ast.Attribute) and node.value.attr == "environ" and environment(literal(node.slice)):
            lines.add(node.lineno)
    return sorted(lines)


def audit_fixture_generators(root):
    paths = sorted(set((root / "scripts").glob("generate*.py"))
                   | set((root / "tests" / "fixtures").rglob("*.py")))
    failures = []
    for path in paths:
        try:
            for line in external_python_calls(path.read_text()):
                failures.append(f"{path.relative_to(root)}:{line}: external FFmpeg fixture hook; use owned generation or an explicit reference benchmark")
        except SyntaxError as error:
            failures.append(f"{path.relative_to(root)}:{error.lineno}: generator source could not be audited: {error.msg}")
    return paths, failures


NATIVE_VALIDATORS = ("validate_gpu.py", "validate_resident.py", "validate_hw_cuda.py", "validate_media.py", "validate_klite_coverage.py", "fetch_klite_samples.py")


def audit_native_validators(root):
    paths = [root / "scripts" / name for name in NATIVE_VALIDATORS]
    failures = []
    for path in paths:
        try:
            for line in external_python_calls(path.read_text()):
                failures.append(f"{path.relative_to(root)}:{line}: external FFmpeg validation hook; move it to an explicit reference benchmark")
        except SyntaxError as error:
            failures.append(f"{path.relative_to(root)}:{error.lineno}: validator source could not be audited: {error.msg}")
        except OSError as error:
            failures.append(f"{path.relative_to(root)}: validator source could not be audited: {error}")
    return paths, failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--target", help="Cargo target triple (defaults to rustc host)")
    parser.add_argument("--production", action="store_true",
                        help="compatibility flag; production CUDA graphs are always audited")
    args = parser.parse_args()
    target = args.target
    if target is None:
        version = subprocess.check_output(["rustc", "-vV"], text=True)
        target = next(line.removeprefix("host: ") for line in version.splitlines()
                      if line.startswith("host: "))
    cases = [
        ("owned media library", ROOT / "crates/fvid-media/Cargo.toml", []),
        ("owned HTTP media library", ROOT / "crates/fvid-media/Cargo.toml", ["--no-default-features", "--features", "http-input"]),
        ("owned CUDA media adapter", ROOT / "crates/fvid-media/Cargo.toml", ["--no-default-features", "--features", "native-cuda"]),
        ("headless", ROOT / "Cargo.toml", ["--no-default-features"]),
        ("player", ROOT / "Cargo.toml", ["--no-default-features", "--features", "player"]),
        ("camera bridge", ROOT / "crates/fvid-camera-ffi/Cargo.toml", []),
        ("production media", ROOT / "Cargo.toml", ["--no-default-features", "--features", "media"]),
        ("production CUDA", ROOT / "Cargo.toml", ["--no-default-features", "--features", "media-cuda"]),
        ("media library CUDA", ROOT / "crates/fvid-media/Cargo.toml", ["--no-default-features", "--features", "cuda-hw"]),
        ("reference-marker library", ROOT / "crates/fvid-media/Cargo.toml", ["--no-default-features", "--features", "legacy-ffmpeg"]),
        ("all production features", ROOT / "Cargo.toml", ["--all-features"]),
    ]
    test_paths, failures = audit_ordinary_tests(ROOT)
    if not failures:
        print(f"ordinary tests: {len(test_paths)} Rust files; no known external FFmpeg test calls", flush=True)
    generator_paths, generator_failures = audit_fixture_generators(ROOT)
    failures.extend(generator_failures)
    if not generator_failures:
        print(f"fixture generators: {len(generator_paths)} Python files; no known external FFmpeg calls", flush=True)
    validator_paths, validator_failures = audit_native_validators(ROOT)
    failures.extend(validator_failures)
    if not validator_failures:
        print(f"native validators: {len(validator_paths)} Python files; no known external FFmpeg calls", flush=True)
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
