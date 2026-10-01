#!/usr/bin/env python3
"""Run all native camera bridge suites; no extension installation."""
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PLATFORM = ROOT / "platform/macos"
FFI = ROOT / "crates/fvid-camera-ffi"


def verify_native_dependencies():
    tree = subprocess.check_output([
        "cargo", "tree", "--locked", "--offline", "--manifest-path",
        str(FFI / "Cargo.toml"), "-e", "normal", "--prefix", "none",
    ], text=True)
    forbidden = [line for line in tree.splitlines()
                 if line.split() and (line.split()[0] == "fvid-media"
                 or line.split()[0].startswith(("ffmpeg", "libav")))]
    if forbidden:
        raise RuntimeError("Camera dependency graph contains foreign media: " + "; ".join(forbidden))


def verify_native_linkage(executable):
    linked = subprocess.check_output(["otool", "-L", str(executable)], text=True)
    dependencies = linked.splitlines()[1:]
    forbidden = [line.strip() for line in dependencies
                 if re.search(r"/(?:libavcodec|libavformat|libavutil|libavfilter|libswscale|libswresample|libavdevice)(?:[.\s]|$)", line)]
    if forbidden:
        raise RuntimeError("Camera executable links FFmpeg: " + "; ".join(forbidden))


def main():
    verify_native_dependencies()
    subprocess.run([
        "cargo", "build", "--locked", "--offline", "--manifest-path", str(FFI / "Cargo.toml")
    ], check=True)
    subprocess.run([
        "cargo", "build", "--locked", "--offline", "--no-default-features",
        "--manifest-path", str(ROOT / "Cargo.toml"), "--target-dir", str(ROOT / "target"),
        "--example", "decode_native_rgb"
    ], check=True)
    cache = ROOT / "target/camera-swift-cache"
    cache.mkdir(parents=True, exist_ok=True)
    common = [
        "xcrun", "swiftc", "-parse-as-library", "-warnings-as-errors",
        "-module-cache-path", str(cache),
        str(PLATFORM / "Shared/CameraFormat.swift"),
        "-import-objc-header", str(PLATFORM / "CameraHost/FVidCamera.h"),
        str(FFI / "target/debug/libfvid_camera_ffi.a"),
    ]
    extension_sources = ["../CameraExtension/PixelPool.swift",
                         "../CameraExtension/CameraProvider.swift",
                         "../CameraExtension/CameraSink.swift"]
    cases = [
        ("CameraClockTests", [], []),
        ("CameraPipelineTests", ["NativeVideoSource.swift", "CameraSession.swift", "CameraProducer.swift",
            "../CameraExtension/PixelPool.swift", "../CameraExtension/CameraProvider.swift",
            "../CameraExtension/CameraSink.swift"], []),
        ("CameraRepeatTests", ["NativeVideoSource.swift"], ["tests/fixtures/video.mp4"]),
        ("CameraAspectTests", ["NativeVideoSource.swift"], ["tests/fixtures/display/par-2x1.mp4"]),
        ("NativeVideoSourceTests", ["NativeVideoSource.swift"], []),
        ("CameraProducerTests", ["CameraProducer.swift"], []),
        ("PixelPoolTests", extension_sources, []),
        ("CameraSinkTests", extension_sources, []),
    ]
    with tempfile.TemporaryDirectory(prefix="fvid-camera-bridge-") as directory:
        for name, sources, fixtures in cases:
            output = Path(directory) / name
            subprocess.run(common + [str(PLATFORM / "CameraHost" / s) for s in sources]
                           + [str(PLATFORM / "Tests" / (name + ".swift")), "-o", str(output)], check=True)
            verify_native_linkage(output)
            subprocess.run([str(output)] + [str(ROOT / f) for f in fixtures], check=True)
        # Compare the FFI BGRA/seek path against a direct software RGB decode.
        # This verifies transport and frame selection; codec conformance has its
        # own independent saved references. No FFmpeg/VideoToolbox is used here.
        fixtures = ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4",
                    "vp9/adaptive.webm", "vp9/odd10.webm", "vp9/lossless12.webm",
                    "av1/ramp.webm", "av1/tiles.webm", "av1/random-access.webm"]
        for index, fixture in enumerate(fixtures):
            source = ROOT / "tests/fixtures" / fixture
            rgb = Path(directory) / f"reference-{index}.rgb"
            times = Path(directory) / f"reference-{index}.json"
            subprocess.run([str(ROOT / "target/debug/examples/decode_native_rgb"),
                            str(source), str(rgb), str(times)], check=True)
            subprocess.run([str(Path(directory) / "NativeVideoSourceTests"),
                            str(source), str(rgb), str(times)], check=True)
    print(f"All {len(cases)} camera bridge suites and {len(fixtures)} AVC/HEVC/VP9/AV1 pixel comparisons passed; dependency and executable linkage checks exclude FFmpeg; installed CMIO delivery is not tested.")


if __name__ == "__main__":
    main()
