#!/usr/bin/env python3
"""Regenerate synthetic one-second clips and independent planar pixel oracles."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/short"
OUT.mkdir(parents=True, exist_ok=True)
CASES = [
    ("avc-baseline.mp4", ["-c:v", "libx264", "-threads", "1", "-profile:v", "baseline", "-pix_fmt", "yuv420p", "-g", "4", "-bf", "0", "-sc_threshold", "0"]),
    ("avc-bframes.mp4", ["-c:v", "libx264", "-threads", "1", "-profile:v", "high", "-pix_fmt", "yuv420p", "-g", "4", "-bf", "2", "-sc_threshold", "0"]),
    ("vp9-motion.webm", ["-c:v", "libvpx-vp9", "-threads", "1", "-lag-in-frames", "0", "-auto-alt-ref", "0", "-g", "4", "-crf", "33", "-b:v", "0"]),
]
for name, args in CASES:
    video = OUT / name
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=96x64:rate=12:duration=1", "-an", *args, "-fflags", "+bitexact", str(video)], check=True)
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(video), "-fps_mode", "passthrough", "-pix_fmt", "yuv420p", "-f", "rawvideo", str(video.with_suffix(".yuv"))], check=True)
