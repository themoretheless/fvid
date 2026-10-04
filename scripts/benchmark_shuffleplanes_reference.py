#!/usr/bin/env python3
"""Explicit FFmpeg oracle benchmark for owned FFV1 fixtures and analytical shuffle."""
import argparse
from pathlib import Path
import subprocess
import time
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ffmpeg',default='ffmpeg')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for name,format,mapping,promote in [
    ('shuffleplanes-444-8','yuv444p','1:2:0:0',False),
    ('shuffleplanes-420-10','yuv420p10le','0:2:1:0',False),
    ('shuffleplanes-420-10-promote','yuv444p10le','1:2:0:0',True),
    ('shuffleplanes-444-16','yuv444p16le','2:2:0:0',False),
]:
    source=(root/(name+'.y4m')).read_bytes()
    body=source[source.index(b'\n')+1:]
    width=1 if '8' in name else 2
    frame_bytes=(64+2*(16 if '420' in name else 64))*width
    original=bytearray()
    for _ in range(2):
        assert body.startswith(b'FRAME\n')
        original+=body[6:6+frame_bytes]
        body=body[6+frame_bytes:]
    assert not body
    native_format='yuv420p10le' if '420' in name else format
    start=time.perf_counter()
    decoded=subprocess.run([args.ffmpeg,'-nostdin','-v','error','-i',str(root/(name+'.mkv')),
        '-pix_fmt',native_format,'-f','rawvideo','pipe:1'],check=True,stdout=subprocess.PIPE).stdout
    assert decoded==original, name+': owned FFV1 differs from original PCM'
    filters=('scale=flags=neighbor,format='+format+',' if promote else '')+'shuffleplanes='+mapping
    shuffled=subprocess.run([args.ffmpeg,'-nostdin','-v','error','-i',str(root/(name+'.y4m')),
        '-vf',filters,'-pix_fmt',format,'-f','rawvideo','pipe:1'],check=True,stdout=subprocess.PIPE).stdout
    assert shuffled==(root/(name+'.yuv')).read_bytes(), name+': analytical shuffle differs'
    expression_mapping=':'.join(f'map{i}={value}*2/2' for i,value in enumerate(mapping.split(':')))
    expression_filters=('scale=flags=neighbor,format='+format+',' if promote else '')+'shuffleplanes='+expression_mapping
    expression_result=subprocess.run([args.ffmpeg,'-nostdin','-v','error','-i',str(root/(name+'.y4m')),
        '-vf',expression_filters,'-pix_fmt',format,'-f','rawvideo','pipe:1'],check=True,stdout=subprocess.PIPE).stdout
    assert expression_result==(root/(name+'.yuv')).read_bytes(), name+': expression shuffle differs'
    print(f'{name}: exact FFV1 decode and shuffle, {(time.perf_counter()-start)*1000:.3f} ms including three process startups')
