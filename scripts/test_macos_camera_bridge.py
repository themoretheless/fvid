#!/usr/bin/env python3
"""Run all native camera bridge suites; no extension installation."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PLATFORM = ROOT / "platform/macos"
FFI = ROOT / "crates/fvid-camera-ffi"


def main():
    subprocess.run([
        "cargo", "build", "--locked", "--offline", "--manifest-path", str(FFI / "Cargo.toml")
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
            subprocess.run([str(output)] + [str(ROOT / f) for f in fixtures], check=True)
    print(f"All {len(cases)} camera bridge suites passed; installed CMIO delivery is not tested.")


if __name__ == "__main__":
    main()
