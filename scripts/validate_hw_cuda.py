#!/usr/bin/env python3
"""Smoke NVDEC → fvid-cuda NV12 filter → NVENC on NVIDIA Windows/Linux."""
import json
import pathlib
import subprocess
import tempfile
from common import prepend_cuda_bin, release_binary

prepend_cuda_bin()
binary = release_binary()


def run(cmd):
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"{cmd}\n{result.stderr}")
    return result


with tempfile.TemporaryDirectory(prefix="fvid-hw-") as temp:
    root = pathlib.Path(temp)
    source = root / "src.mp4"
    output = root / "out.mp4"
    run([
        "ffmpeg", "-nostdin", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25",
        "-t", "1", "-an", "-c:v", "libx264", "-pix_fmt", "yuv420p", str(source),
    ])
    result = run([
        str(binary), "media", "hw-filter", str(source), str(output),
        "--crop", "16:16:320:180", "--hflip", "--vflip",
    ])
    stats = json.loads(result.stdout)
    assert stats["backend"] == "cuda-nvdec-nvenc"
    assert stats["host_frame_copies"] == 0
    assert stats["device_filter_passes"] == 25
    assert stats["video_frames"] == 25
    assert stats["width"] == 320 and stats["height"] == 180
    run(["ffmpeg", "-nostdin", "-v", "error", "-i", str(output), "-f", "null", "-"])
    print("hw-filter smoke passed:", stats)
