#!/usr/bin/env python3
"""Explicit external reference fixture generation for physical CUDA acceptance.

Ordinary tests consume the saved synthetic files and never invoke this script.
The 256x192 canvases meet Blackwell's NVDEC/NVENC geometry requirements.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from generate_playback_error_samples import rewrite

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write-fixtures', action='store_true', required=True)
    parser.add_argument('--ffmpeg', default='ffmpeg')
    args = parser.parse_args()
    output = ROOT / 'tests/fixtures/playback-errors'
    encoded = {}
    for name, codec, depth, width, height, rate, count in [
        ('cuda-h264', 'libx264', 8, 256, 192, 12, 12),
        ('cuda-h264-crop', 'libx264', 8, 318, 238, 30, 8),
        ('cuda-hevc', 'libx265', 8, 256, 192, 30, 17),
        ('cuda-hevc-main10', 'libx265', 10, 256, 192, 30, 17),
    ]:
        destination = output / (name + '.mp4')
        command = [args.ffmpeg, '-hide_banner', '-loglevel', 'error', '-y',
                   '-f', 'lavfi', '-i', f'testsrc2=size={width}x{height}:rate={rate}',
                   '-frames:v', str(count), '-an', '-c:v', codec,
                   '-pix_fmt', 'yuv420p10le' if depth == 10 else 'yuv420p',
                   '-threads', '1']
        if codec == 'libx264':
            command += ['-profile:v', 'main', '-x264-params',
                        'bframes=2:b-adapt=0:ref=3:keyint=30:scenecut=0:threads=1']
        else:
            command += ['-x265-params',
                        'bframes=2:b-adapt=0:ref=3:keyint=30:min-keyint=30:scenecut=0:open-gop=0:wpp=0:pools=none:frame-threads=1:log-level=error']
        command += ['-video_track_timescale', '15360' if codec == 'libx265' else '12000',
                    '-movie_timescale', '30' if codec == 'libx265' else '1000', str(destination)]
        subprocess.run(command, check=True)
        encoded[name] = destination.read_bytes()
    for name in ('cuda-hevc', 'cuda-hevc-main10'):
        (output / (name + '-edit-repeat.mp4')).write_bytes(rewrite(
            encoded[name], edits=[(3, -1), (6, 1024), (3, -1), (6, 1024)]))
    (output / 'cuda-h264-edit-empty-spans.mov').write_bytes(rewrite(
        encoded['cuda-h264'], edits=[(1000, -1), (2000, 0), (1000, -1), (2000, 0)]))
    (output / 'cuda-h264-video-metadata.mp4').write_bytes(rewrite(
        encoded['cuda-h264'], video_metadata=True))
    inventory = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(output.glob('cuda-*.mp4'))}
    inventory['cuda-h264-edit-empty-spans.mov'] = hashlib.sha256(
        (output / 'cuda-h264-edit-empty-spans.mov').read_bytes()).hexdigest()
    (output / 'cuda-hardware-fixtures.json').write_text(json.dumps(inventory, indent=2) + '\n')


if __name__ == '__main__':
    main()
