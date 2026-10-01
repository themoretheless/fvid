#!/usr/bin/env python3
"""Generate a synthetic HEVC camera resolution-change regression (reference tools only)."""
from pathlib import Path
import subprocess
import tempfile

fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
with tempfile.TemporaryDirectory(prefix='fvid-camera-resize-') as temp:
    first, second, joined = [Path(temp) / name for name in ('first.hevc', 'second.hevc', 'joined.hevc')]
    subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
        'testsrc2=size=128x128:rate=30:duration=0.1', '-c:v', 'libx265',
        '-pix_fmt', 'yuv420p', '-x265-params',
        'pools=1:frame-threads=1:ctu=32:bframes=0:log-level=error',
        '-an', '-f', 'hevc', str(first)], check=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(fixtures / 'hevc-sps-resize.mp4'),
        '-c', 'copy', '-bsf:v', 'hevc_mp4toannexb', '-f', 'hevc', str(second)], check=True)
    joined.write_bytes(first.read_bytes() + second.read_bytes())
    subprocess.run(['ffmpeg', '-v', 'error', '-r', '30', '-i', str(joined),
        '-c', 'copy', '-tag:v', 'hev1', '-y', str(fixtures / 'hevc-camera-resize.mp4')], check=True)
