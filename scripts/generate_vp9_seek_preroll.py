#!/usr/bin/env python3
"""Generate an owned long-GOP VP9 seek fixture; not invoked by tests."""
from pathlib import Path
import subprocess
output = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/vp9-long-gop-seek.webm'
subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'testsrc2=size=96x64:rate=30:duration=10.2',
    '-an', '-c:v', 'libvpx-vp9', '-threads', '1', '-cpu-used', '8',
    '-g', '102', '-keyint_min', '102', '-pix_fmt', 'yuv420p', str(output)], check=True)
