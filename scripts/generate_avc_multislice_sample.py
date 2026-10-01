#!/usr/bin/env python3
"""Synthetic two-slice AVC reference. FFmpeg/x264 run only at generation time."""
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
video = root / 'avc-multislice-ipb.mp4'
reference = root / 'avc-multislice-ipb.yuv'
subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=size=128x96:rate=30:duration=0.2666667',
    '-frames:v','8','-c:v','libx264','-pix_fmt','yuv420p','-crf','18',
    '-x264-params','threads=1:slices=2:keyint=30:bframes=2:ref=3','-an','-y',str(video)],check=True)
subprocess.run(['ffmpeg','-v','error','-i',str(video),'-pix_fmt','yuv420p','-f','rawvideo','-y',str(reference)],check=True)
for name, pixel_format, cabac in [
    ('avc-multislice-cavlc', 'yuv420p', 0),
    ('avc-multislice-cabac10', 'yuv420p10le', 1),
    ('avc-multislice-cavlc10', 'yuv420p10le', 0),
]:
    video = root / (name + '.mp4')
    reference = root / (name + '.yuv')
    subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
        'testsrc2=size=128x96:rate=30:duration=0.2666667', '-frames:v', '8',
        '-c:v', 'libx264', '-pix_fmt', pixel_format, '-crf', '18', '-x264-params',
        f'threads=1:slices=2:keyint=30:bframes=2:ref=3:cabac={cabac}',
        '-an', '-y', str(video)], check=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(video), '-pix_fmt', pixel_format,
        '-f', 'rawvideo', '-y', str(reference)], check=True)
