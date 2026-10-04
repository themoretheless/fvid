#!/usr/bin/env python3
"""Explicit adapter-domain probe; never run by ordinary tests."""
from pathlib import Path
import os, struct, subprocess
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8, 12, 16):
    source = root / f'grayworld-gamut-{depth}.y4m'
    raw = subprocess.run([os.environ.get('FVID_FFMPEG', 'ffmpeg'), '-nostdin', '-v', 'error', '-filter_threads', '1', '-i', str(source), '-vf', 'format=gbrpf32le', '-frames:v', '3', '-pix_fmt', 'gbrpf32le', '-f', 'rawvideo', 'pipe:1'], check=True, capture_output=True).stdout
    values = struct.unpack('<' + 'f' * (len(raw) // 4), raw)
    print(f'depth={depth} components={len(values)} min={min(values)} max={max(values)}')
    assert min(values) >= 0 and max(values) <= 1
