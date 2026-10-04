#!/usr/bin/env python3
"""Synthetic cropped AVC; x264 fixture encoder and own MP4 muxer, no FFmpeg."""
import argparse
from pathlib import Path
import subprocess
import tempfile
from avc_fixture_mp4 import read_mkv, mux


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--x264', default='x264')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='fvid-avc-crop-') as temporary:
        directory = Path(temporary)
        raw, encoded = directory / 'source.yuv', directory / 'source.mkv'
        width, height, count = 62, 46, 8
        pixels = bytearray()
        for frame in range(count):
            for plane in range(3):
                w, h = (width, height) if plane == 0 else (width // 2, height // 2)
                pixels.extend(24 + (x * 3 + y * 5 + frame * 17 + plane * 29) % 160
                              for y in range(h) for x in range(w))
        raw.write_bytes(pixels)
        subprocess.run([args.x264, '--demuxer', 'raw', '--input-csp', 'i420', '--input-res', f'{width}x{height}',
                        '--fps', '30', '--frames', str(count), '--threads', '1', '--keyint', '30',
                        '--bframes', '2', '--b-adapt', '0', '--ref', '3', '--profile', 'main', '--crf', '18',
                        '--muxer', 'mkv', '-o', str(encoded), str(raw)], check=True)
        configuration, frames = read_mkv(encoded.read_bytes(), 30)
        assert len(frames) == count
        (args.output / 'avc-display-crop.mp4').write_bytes(mux(configuration, frames, width, height, 30))


if __name__ == '__main__':
    main()
