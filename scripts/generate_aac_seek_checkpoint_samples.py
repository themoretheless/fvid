#!/usr/bin/env python3
"""Remux committed synthetic AAC into seek fixtures. Not invoked by tests."""
from pathlib import Path
import subprocess
root=Path(__file__).resolve().parents[1]
source=root/'tests/fixtures/audio/two-audio.mp4'
output=root/'tests/fixtures/playback-errors'
for extension in ['aac','mka']:
    subprocess.run(['ffmpeg','-v','error','-i',str(source),'-map','0:a:0','-c:a','copy',
                    '-avoid_negative_ts','make_zero','-y',str(output/('aac-seek-checkpoints.'+extension))],check=True)
