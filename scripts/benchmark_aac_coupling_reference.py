#!/usr/bin/env python3
"""Explicit FFmpeg comparison of analytical AAC CCE oracles; not an ordinary test."""
import argparse
from pathlib import Path
import struct
import subprocess
import time
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ffmpeg',default='ffmpeg')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for path in sorted(root.glob('aac-independent-coupling*.aac')):
    if 'missing-target' in path.name:
        continue
    start=time.perf_counter()
    reference=subprocess.run([args.ffmpeg,'-nostdin','-v','error','-i',str(path),'-f','f32le','-c:a','pcm_f32le','pipe:1'],check=True,stdout=subprocess.PIPE).stdout
    expected=path.with_suffix('.f32le').read_bytes()
    assert len(reference)==len(expected)
    count=len(expected)//4
    actual=struct.unpack('<'+'f'*count,reference)
    analytic=struct.unpack('<'+'f'*count,expected)
    peak=max(abs(a-b) for a,b in zip(actual,analytic))
    assert peak<1e-7, (path.name,peak)
    print(f'{path.stem}: peak error {peak:.3g}, {(time.perf_counter()-start)*1000:.3f} ms including process startup')
