#!/usr/bin/env python3
"""Duplicate synthetic ALAC tracks; FFmpeg is a fixture-generation tool only."""
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1]
subprocess.run(['ffmpeg', '-v', 'error', '-i', str(root / 'tests/fixtures/alac/stereo-24.m4a'),
    '-map', '0:a:0', '-map', '0:a:0', '-c', 'copy', '-y',
    str(root / 'tests/fixtures/playback-errors/alac-two-tracks.m4a')], check=True)
