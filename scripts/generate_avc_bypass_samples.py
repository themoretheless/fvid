#!/usr/bin/env python3
"""Synthetic lossless AVC fixtures; FFmpeg/x264 are reference tools only."""
from pathlib import Path
import subprocess
import tempfile

fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
with tempfile.TemporaryDirectory(prefix='fvid-avc-bypass-') as temp:
    original = Path(temp) / 'original.yuv'
    pattern = 'testsrc2=size=64x64:rate=30:duration=0.2666667'
    subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i',pattern,'-frames:v','8',
        '-pix_fmt','yuv420p','-f','rawvideo',str(original)],check=True)
    for name, coder, pixel in [('lossless','1','yuv420p'),('cavlc','0','yuv420p'),('main10','1','yuv420p10le')]:
        subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i',pattern,'-frames:v','8',
            '-pix_fmt',pixel,'-f','rawvideo','-y',str(original)],check=True)
        video = fixtures / f'avc-bypass-{name}.mp4'
        reference = fixtures / f'avc-bypass-{name}.yuv'
        subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i',pattern,'-frames:v','8',
            '-c:v','libx264','-pix_fmt',pixel,'-qp','0','-coder',coder,
            '-x264-params','threads=1:keyint=30:bframes=2:ref=3',
            '-an','-y',str(video)],check=True)
        subprocess.run(['ffmpeg','-v','error','-i',str(video),'-pix_fmt',pixel,
            '-f','rawvideo','-y',str(reference)],check=True)
        assert reference.read_bytes() == original.read_bytes(), 'reference decode is not lossless'
