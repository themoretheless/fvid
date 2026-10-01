#!/usr/bin/env python3
"""Generate synthetic AVC scaling matrices and independent YUV fixtures."""
from pathlib import Path
import subprocess

fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
custom = []
for index, (name, count) in enumerate((('cqm4iy',16),('cqm4ic',16),('cqm4py',16),('cqm4pc',16),('cqm8i',64),('cqm8p',64))):
    weights = ','.join(str(7 + index * 2 + (i * 5 + i // 4) % 23) for i in range(count))
    custom.append(f'{name}={weights}')
for name, matrix in [('jvt','cqm=jvt'),('custom',':'.join(custom))]:
    source = fixtures / f'avc-scaling-{name}.mp4'
    reference = fixtures / f'avc-scaling-{name}.yuv'
    subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i',
        'testsrc2=size=64x64:rate=30:duration=0.2666667','-frames:v','8',
        '-c:v','libx264','-profile:v','high','-pix_fmt','yuv420p','-x264-params',
        f'threads=1:keyint=30:bframes=2:ref=3:partitions=i8x8,p8x8,b8x8:{matrix}','-an','-y',str(source)],check=True)
    subprocess.run(['ffmpeg','-v','error','-i',str(source),'-pix_fmt','yuv420p',
        '-f','rawvideo','-y',str(reference)],check=True)
    assert reference.stat().st_size == 8 * 64 * 64 * 3 // 2
