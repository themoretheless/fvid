#!/usr/bin/env python3
"""Remux committed synthetic AAC with owned framing. Not invoked by tests."""
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1]
subprocess.run(['cargo', 'run', '--no-default-features', '--example', 'aac_seek_fixture',
                '--', str(root / 'tests/fixtures/audio/two-audio.mp4'),
                str(root / 'tests/fixtures/playback-errors')], cwd=root, check=True)
