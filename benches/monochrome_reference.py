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
cases = ('', 'cb=0.5:cr=-0.5:size=0.2:high=0.75',
         'cb=-1:cr=1:size=0.1:high=0', 'cb=0.25:cr=0.4:size=10:high=1',
         'cb=0:cr=0:size=1:high=0.5')
def run(command):
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return result.stdout
with tempfile.TemporaryDirectory(prefix='fvid-monochrome-reference-') as scratch:
    for depth in (8, 12, 16):
        source = root / f'tests/fixtures/playback-errors/monochrome-grid-{depth}.y4m'
        pix_fmt = 'yuv420p' if depth == 8 else f'yuv420p{depth}le'
        for index, case in enumerate(cases):
            output = Path(scratch) / f'{depth}-{index}.mkv'
            run([str(args.fvid.resolve()), 'media', 'transcode-lossless', str(source), str(output), '--monochrome', case])
            expected = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source), '-vf', f'monochrome={case}', '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            actual = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(output), '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            assert len(actual) == len(expected)
            width = 1 if depth == 8 else 2
            values = lambda payload: [int.from_bytes(payload[at:at+width], 'little') for at in range(0, len(payload), width)]
            pairs = list(zip(values(actual), values(expected)))
            differences = sum(a != b for a, b in pairs)
            maximum = max(abs(a-b) for a, b in pairs)
            print(f'depth={depth} case={case!r} samples={len(pairs)} different={differences} max_delta={maximum}')
            assert differences == 0, 'monochrome differs from reference'
