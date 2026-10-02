#!/usr/bin/env python3
"""Explicit FFmpeg/JM temporal-direct reference benchmark; not an ordinary test."""
import argparse
from pathlib import Path
import subprocess
import time
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ffmpeg',default='ffmpeg')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
reference=(root/'avc-slice-lists-temporal-jm.yuv').read_bytes()
results=[]
for threads in [1,0]:
    start=time.perf_counter()
    result=subprocess.run([args.ffmpeg,'-nostdin','-v','error','-threads',str(threads),
        '-i',str(root/'avc-slice-lists-temporal.h264'),'-fps_mode','passthrough',
        '-pix_fmt','yuv420p','-f','rawvideo','pipe:1'],check=True,stdout=subprocess.PIPE)
    elapsed=time.perf_counter()-start
    actual=result.stdout
    assert len(actual)==len(reference), 'reference frame count mismatch'
    mismatch=sum(a!=b for a,b in zip(actual,reference))
    print(f'threads={threads}: {elapsed*1000:.3f} ms including process startup, {mismatch}/{len(reference)} samples differ from JM')
    results.append(actual)
print('Thread outputs identical:',results[0]==results[1])
