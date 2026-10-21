#!/usr/bin/env python3
"""Explicit external reference refresh, outside ordinary fixture generation.
Encodes only fixed synthetic tone/video. Output defaults to ignored benchmark
data; --output can explicitly replace checked-in frozen vectors. This is not a
claim that native encoders for these six audio formats are implemented.
"""
import argparse
import subprocess
import sys
import tempfile
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from matroska_sample_clock_reference import capture, ENCODERS

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ffmpeg', default='ffmpeg')
    parser.add_argument('--output', type=Path, default=ROOT / 'benchmarks/data/matroska-sample-clock-reference')
    args = parser.parse_args(); movies = {}
    with tempfile.TemporaryDirectory(prefix='fvid-audio-clock-reference-') as directory:
        for name, encoder in ENCODERS:
            path = Path(directory) / (name + '.mkv')
            subprocess.run([args.ffmpeg, '-hide_banner', '-loglevel', 'error', '-y',
                '-f', 'lavfi', '-i', 'color=c=gray:s=16x16:r=50:d=0.24',
                '-f', 'lavfi', '-i', 'aevalsrc=0.25*sin(2*PI*440*t):s=48000:d=0.24',
                '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'libvpx-vp9', '-lossless', '1', '-threads', '1',
                '-c:a', encoder, '-strict', 'experimental', '-ar', '48000', '-ac', '2' if name == 'vorbis' else '1',
                '-t', '0.24', *(['-b:a', '128k'] if name == 'mp2' else []), str(path)], check=True)
            movies[name] = path.read_bytes()
    files = capture(movies); args.output.mkdir(parents=True, exist_ok=True)
    for name, data in files.items(): (args.output / name).write_bytes(data)

if __name__ == '__main__': main()
