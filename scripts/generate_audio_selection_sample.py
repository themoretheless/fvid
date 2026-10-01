#!/usr/bin/env python3
"""Duplicate synthetic ALAC tracks; FFmpeg is a fixture-generation tool only."""
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1]
subprocess.run(['ffmpeg', '-v', 'error', '-i', str(root / 'tests/fixtures/alac/stereo-24.m4a'),
    '-map', '0:a:0', '-map', '0:a:0', '-c', 'copy', '-y',
    str(root / 'tests/fixtures/playback-errors/alac-two-tracks.m4a')], check=True)
# Different sample rates/channel counts make accidental default-track selection visible.
subprocess.run(['ffmpeg', '-v', 'error', '-i', str(root / 'tests/fixtures/audio/aac-mono-44k.aac'),
    '-i', str(root / 'tests/fixtures/audio/aac-stereo.mka'), '-map', '0:a:0', '-map', '1:a:0',
    '-c', 'copy', '-y', str(root / 'tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a')], check=True)

subprocess.run(['ffmpeg', '-v', 'error', '-i', str(root / 'tests/fixtures/audio/aac-mono-44k.aac'),
    '-f', 'lavfi', '-i', 'sine=frequency=997:sample_rate=48000:duration=0.15',
    '-map', '0:a:0', '-map', '1:a:0', '-c:a:0', 'copy', '-c:a:1', 'aac',
    '-ac:a:1', '2', '-y', str(root / 'tests/fixtures/playback-errors/aac-two-tracks.m4a')], check=True)
