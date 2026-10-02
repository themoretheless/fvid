#!/usr/bin/env python3
"""Differential GPU checks and optional, reproducible end-to-end CLI benchmarks.

Writes a new GPU report; never changes the earlier CPU benchmark snapshots.
An unavailable backend is recorded, never counted as an exercised GPU.
"""
import argparse
import datetime
import hashlib
import io
import json
import pathlib
import platform
import re
import subprocess
import tempfile

from common import ROOT, prepend_cuda_bin, release_binary
from y4m_oracle import transform as reference_transform

FORMATS = {"420": (2, 2, "yuv420p"), "422": (2, 1, "yuv422p"), "444": (1, 1, "yuv444p")}
def invoke(command, **kwargs):
    return subprocess.run(command, stderr=subprocess.PIPE, timeout=300, **kwargs)


def success(command, **kwargs):
    result = invoke(command, **kwargs)
    if result.returncode:
        raise RuntimeError(f"{command}: {result.stderr.decode(errors='replace')}")
    return result


def signature(data):
    stream = io.BytesIO(data)
    header = stream.readline()
    width = int(re.search(rb" W(\d+)", header)[1])
    height = int(re.search(rb" H(\d+)", header)[1])
    chroma = re.search(rb" C(420|422|444)", header)[1].decode()
    sx, sy, _ = FORMATS[chroma]
    size = width * height + 2 * (width // sx) * (height // sy)
    hashes = []
    while marker := stream.readline():
        if not marker.startswith(b"FRAME"):
            raise ValueError(f"Bad frame marker: {marker[:30]!r}")
        frame = stream.read(size)
        if len(frame) != size:
            raise ValueError("Truncated output frame")
        hashes.append(hashlib.sha256(frame).hexdigest())
    return dict(width=width, height=height, chroma=chroma, frames=len(hashes), frame_sha256=hashes)


def make_input(path, width, height, chroma, frames):
    sx, sy, _ = FORMATS[chroma]
    size = width * height + 2 * (width // sx) * (height // sy)
    # Non-uniform per-plane/pixel data and adjacent repeated frames catch stale
    # readback, row-pitch, byte-lane and buffer-reuse mistakes.
    with path.open("wb") as output:
        output.write(f"YUV4MPEG2 W{width} H{height} F30:1 Ip C{chroma}\n".encode())
        for frame in range(frames):
            block = bytes((i * 37 + (i // width) * 19 + (frame // 2) * 53) % 256 for i in range(min(size, 65536)))
            output.write(b"FRAME\n")
            output.write((block * ((size + len(block) - 1) // len(block)))[:size])


def options(crop, horizontal, vertical):
    args = ["--crop", ":".join(map(str, crop))] if crop else []
    return args + (["--hflip"] if horizontal else []) + (["--vflip"] if vertical else [])


def selected_backend(stderr):
    values = re.findall(r"\bbackend=(cpu|metal|vulkan|dx12|gl|cuda)\b", stderr)
    if not values:
        raise RuntimeError("Successful invocation did not report its actual backend")
    return values[-1]


def file_sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_files():
    """Only authored production inputs, excluding nested target directories."""
    package_roots = [ROOT] + sorted(path for path in (ROOT / "crates").glob("*") if path.is_dir())
    files = []
    for package in package_roots:
        files += [path for name in ("Cargo.toml", "Cargo.lock") if (path := package / name).is_file()]
        files += [path for path in (package / "src").rglob("*") if path.is_file() and path.suffix in (".rs", ".cu", ".wgsl")]
    return sorted(files)


def expected_failure(command, output):
    result = invoke(command, stdout=subprocess.PIPE)
    if result.returncode == 0 or output.exists():
        raise RuntimeError(f"Invalid request succeeded or published output: {command}")
    return dict(command=command, exit_code=result.returncode, stderr=result.stderr.decode(errors="replace"))


def validate(args, work, report):
    device_list = success([args.binary, "--list-devices"], stdout=subprocess.PIPE)
    listing = device_list.stdout.decode(errors="replace")
    report["device_listing"] = listing
    report["device_listing_stderr"] = device_list.stderr.decode(errors="replace")
    probe = work / "probe.y4m"
    make_input(probe, 10, 6, "420", 5)
    report["backends"] = {}
    active = {"cpu": "cpu"}
    for requested in args.backends:
        if requested == "cpu":
            continue
        command = [args.binary, str(probe), "-", "--backend", requested, "--device", str(args.device)]
        result = invoke(command, stdout=subprocess.PIPE)
        stderr = result.stderr.decode(errors="replace")
        record = dict(command=command, exit_code=result.returncode, stderr=stderr)
        report["backends"][requested] = record
        if result.returncode:
            advertised = re.search(rf"backend={re.escape(requested)}\s+device={args.device}\b[^\n]*status=available", listing)
            explicitly_unavailable = re.search(rf"backend={re.escape(requested)}\b[^\n]*status=unavailable", listing)
            other_devices = re.search(rf"backend={re.escape(requested)}\s+device=\d+\b[^\n]*status=available", listing)
            if advertised or not (explicitly_unavailable or other_devices):
                raise RuntimeError(f"Backend {requested} failed without an explicit unavailable listing: {stderr}")
            record["status"] = "unavailable"
            continue
        actual = selected_backend(stderr)
        if requested != "auto" and actual != requested:
            raise RuntimeError(f"Explicit {requested} silently executed on {actual}")
        record.update(status="cpu_fallback" if actual == "cpu" else "exercised", selected_backend=actual)
        active[requested] = actual

    report["correctness"] = []
    configurations = [("420", 10, 6, (2, 2, 6, 2)), ("422", 10, 5, (2, 1, 6, 3)), ("444", 9, 7, (1, 1, 5, 3))]
    for chroma, width, height, rectangle in configurations:
        source = work / f"small-{chroma}.y4m"
        make_input(source, width, height, chroma, 9)
        for crop in (None, rectangle):
            for horizontal, vertical in ((False, False), (True, False), (False, True), (True, True)):
                case = dict(chroma=chroma, input_dimensions=[width, height], crop=crop, horizontal=horizontal, vertical=vertical)
                commands = {backend: [args.binary, str(source), "-", "--backend", backend, "--device", str(args.device if backend != "cpu" else 0)] + options(crop, horizontal, vertical) for backend in active}
                signatures, diagnostics = {"python_oracle": signature(reference_transform(source.read_bytes(), crop, horizontal, vertical))}, {}
                cpu_bytes = None
                for engine, command in commands.items():
                    result = success(command, stdout=subprocess.PIPE)
                    if engine == "cpu":
                        cpu_bytes = result.stdout
                    elif result.stdout != cpu_bytes:
                        raise RuntimeError(f"Y4M metadata or frame bytes differ from CPU: {case}, {engine}")
                    signatures[engine] = signature(result.stdout)
                    diagnostics[engine] = result.stderr.decode(errors="replace")
                    if selected_backend(diagnostics[engine]) != active[engine]:
                        raise RuntimeError(f"Backend changed during checks: {engine}")
                if any(value != signatures["cpu"] for value in signatures.values()):
                    raise RuntimeError(f"Frame mismatch: {case}: {signatures}")
                case.update(signature=signatures["cpu"], verified_engines=["python_oracle", *commands], diagnostics=diagnostics)
                report["correctness"].append(case)
        print(f"Correctness: {chroma}, 8 transform cases, engines={','.join(active)},python_oracle", flush=True)

    # Cross texture-row, dispatch and large-frame boundaries as well as the
    # tiny packing cases above. Benchmarks alone do not verify output bytes.
    report["large_frame_correctness"] = []
    for chroma, width, height, rectangle in [
        ("444", 65, 17, (1, 1, 63, 15)),
        ("444", 257, 129, (1, 1, 255, 127)),
        ("422", 638, 479, (2, 1, 634, 477)),
        ("420", 1280, 720, (160, 90, 640, 360)),
        ("420", 3840, 2160, (480, 270, 1920, 1080)),
    ]:
        source = work / f"boundary-{width}-{height}-{chroma}.y4m"
        make_input(source, width, height, chroma, 2)
        for crop in (None, rectangle):
            reference = signature(reference_transform(source.read_bytes(), crop, True, True))
            case = dict(chroma=chroma, input_dimensions=[width, height], crop=crop, horizontal=True, vertical=True, verified_engines=["python_oracle"])
            for engine in active:
                command = [args.binary, str(source), "-", "--backend", engine, "--device", str(args.device if engine != "cpu" else 0)] + options(crop, True, True)
                result = success(command, stdout=subprocess.PIPE)
                actual_signature = signature(result.stdout)
                if reference is not None and actual_signature != reference:
                    raise RuntimeError(f"Large-frame output mismatch: {case}, {engine}")
                if selected_backend(result.stderr.decode(errors="replace")) != active[engine]:
                    raise RuntimeError(f"Backend changed during large-frame checks: {engine}")
                reference = actual_signature
                case["verified_engines"].append(engine)
            case["signature"] = reference
            report["large_frame_correctness"].append(case)
        source.unlink()
    print("Large-frame/texture-boundary correctness: 10 cases passed", flush=True)

    report["failure_checks"] = []
    output = work / "must-not-exist.y4m"
    invalid = [["--backend", "does-not-exist"], ["--backend", "cpu", "--device", "not-a-number"]]
    for backend in args.backends:
        if backend not in ("auto", "cpu"):
            invalid.append(["--backend", backend, "--device", "4294967295"])
    for backend in active:
        invalid.append(["--backend", backend, "--memory-mib", "0"])
    for flags in invalid:
        report["failure_checks"].append(expected_failure([args.binary, str(probe), str(output)] + flags, output))

    broken = work / "truncated.y4m"
    broken.write_bytes(probe.read_bytes()[:-1])
    for backend in active:
        report["failure_checks"].append(expected_failure([args.binary, str(broken), str(output), "--backend", backend], output))

    # CPU input+output fits in 1 MiB, but GPU textures/staging/readback add
    # explicitly controlled allocations. This catches budgeting only the CPU
    # frames while quietly exceeding the user's GPU processing budget.
    budget_input = work / "memory-bound.y4m"
    make_input(budget_input, 512, 512, "420", 2)
    success([args.binary, str(budget_input), "-", "--backend", "cpu", "--memory-mib", "1"], stdout=subprocess.DEVNULL)
    for backend, actual in active.items():
        if actual != "cpu" and backend != "auto":
            report["failure_checks"].append(expected_failure([args.binary, str(budget_input), str(output), "--backend", backend, "--device", str(args.device), "--memory-mib", "1"], output))
    if list(work.glob(".fvid-*.tmp")):
        raise RuntimeError("Failed processing leaked a staging file")
    print(f"Failure/atomic-publication checks: {len(report['failure_checks'])} passed", flush=True)
    return active



def save(path, report):
    destination = pathlib.Path(path)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=str(release_binary()))
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--backends", nargs="+", default=["auto", "metal", "vulkan", "dx12", "gl", "cuda"])
    parser.add_argument("--device", type=int, default=0)
    parser.add_argument("--benchmark", action="store_true")
    parser.add_argument("--resolutions", nargs="+", choices=["720", "1080", "4k"], default=["720", "1080", "4k"])
    parser.add_argument("--frames", type=int, default=60)
    parser.add_argument("--rounds", type=int, default=7)
    parser.add_argument("--report", default=str(ROOT / "benchmarks/gpu-results.json"))
    args = parser.parse_args()
    prepend_cuda_bin()
    if args.frames < 1 or args.rounds < 1 or args.device < 0:
        parser.error("frames and rounds must be positive; device must be non-negative")
    report = dict(validation_reference_sha256=file_sha256(ROOT / "scripts/y4m_oracle.py"), created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(), machine=platform.machine(), binary=args.binary, binary_sha256=file_sha256(args.binary), validation_script_sha256_at_execution=file_sha256(__file__), source_sha256={str(source.relative_to(ROOT)): file_sha256(source) for source in source_files()}, correctness_reference="independent Python planar oracle", rustc=success(["rustc", "--version"], stdout=subprocess.PIPE).stdout.decode().strip(), method="End-to-end CLI wall time including process startup, GPU device/pipeline initialization, file reads, upload, transform, readback and Y4M serialization to OS null sink. One warmup then seeded randomized rounds. Warm file cache. No codec or steady-state resident-GPU performance claim.", benchmark_rounds=args.rounds, status="running")
    try:
        with tempfile.TemporaryDirectory(prefix="fvid-gpu-validation-") as directory:
            work = pathlib.Path(directory)
            active = validate(args, work, report)
            if args.benchmark:
                from benchmark_gpu_reference import run_benchmarks
                run_benchmarks(args, work, active, report)
        report["status"] = "passed"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        save(args.report, report)
    print(f"Saved {args.report}", flush=True)


if __name__ == "__main__":
    main()
