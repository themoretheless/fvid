#!/usr/bin/env python3
"""Explicit external-reference qualification; never invoked by ordinary tests."""
import argparse
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--fvid', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
cases = ('', 'hue=0:saturation=1:lightness=0.5:mix=1',
         'hue=120:saturation=0.5:lightness=0.5:mix=0.5',
         'hue=240:saturation=0.8:lightness=0.2:mix=0',
         'hue=35:saturation=0.72:lightness=0.6:mix=0.3')
def run(command):
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return result.stdout
with tempfile.TemporaryDirectory(prefix='fvid-colorize-reference-') as scratch:
    for depth in (8, 12, 16):
        source = root / f'tests/fixtures/playback-errors/colorize-grid-{depth}.y4m'
        pix_fmt = 'yuv420p' if depth == 8 else f'yuv420p{depth}le'
        for index, case in enumerate(cases):
            output = Path(scratch) / f'{depth}-{index}.mkv'
            run([str(args.fvid.resolve()), 'media', 'transcode-lossless', str(source), str(output), '--colorize', case])
            expected = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source), '-vf', f'colorize={case}', '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            actual = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(output), '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            assert len(actual) == len(expected)
            width = 1 if depth == 8 else 2
            values = lambda payload: [int.from_bytes(payload[at:at+width], 'little') for at in range(0, len(payload), width)]
            pairs = list(zip(values(actual), values(expected)))
            differences = sum(a != b for a, b in pairs)
            maximum = max(abs(a-b) for a, b in pairs)
            print(f'depth={depth} case={case!r} samples={len(pairs)} different={differences} max_delta={maximum}')
            assert differences == 0, 'colorize differs from reference'
