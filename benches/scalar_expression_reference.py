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
cases = (('--colorize','colorize','hue=60*2:saturation=1/2:lightness=1/2:mix=1/2'),
         ('--monochrome','monochrome','cb=1/2:cr=-1/2:size=1/5:high=3/4'),
         ('--eq','eq','gamma=sqrt(4):brightness=1/10'),
         ('--hue','hue','h=90*2:s=1/2'),
         ('--lutyuv','lutyuv','y=gammaval(2):u=clip(val):v=gammaval709(2)'))
def run(command):
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return result.stdout
with tempfile.TemporaryDirectory(prefix='fvid-expression-reference-') as scratch:
    for depth in (8, 12, 16):
        source = root / f'tests/fixtures/playback-errors/colorize-grid-{depth}.y4m'
        pix_fmt = 'yuv420p' if depth == 8 else f'yuv420p{depth}le'
        for index, (flag, name, case) in enumerate(cases):
            output = Path(scratch) / f'{depth}-{index}.mkv'
            run([str(args.fvid.resolve()), 'media', 'transcode-lossless', str(source), str(output), flag, case])
            # Installed AVExpr reserves clip for its three-argument builtin;
            # clipval is the equivalent lookup result for clip(val).
            reference_case = case.replace('clip(val)', 'clipval') if name == 'lutyuv' else case
            expected = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source), '-vf', f'{name}={reference_case}', '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            actual = run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(output), '-an', '-pix_fmt', pix_fmt, '-f', 'rawvideo', '-'])
            assert len(actual) == len(expected)
            width = 1 if depth == 8 else 2
            values = lambda payload: [int.from_bytes(payload[at:at+width], 'little') for at in range(0, len(payload), width)]
            pairs = list(zip(values(actual), values(expected)))
            differences = sum(a != b for a, b in pairs)
            maximum = max(abs(a-b) for a, b in pairs)
            print(f'depth={depth} filter={name} case={case!r} samples={len(pairs)} different={differences} max_delta={maximum}')
            if name in ('eq', 'hue') and depth > (8 if name == 'eq' else 10):
                # vf_eq admits 8-bit, vf_hue admits 8/10-bit: conversion changes
                # the reference pipeline. Verify that distinction explicitly.
                reference_depth = 8 if name == 'eq' else 10
                quantum = 1 << (depth - reference_depth)
                assert all(b % quantum == 0 for _, b in pairs), 'unexpected reference precision'
                assert any(a % quantum != 0 for a, _ in pairs), 'owned filter lost source precision'
                print(f'qualification=reference-{reference_depth}-bit-conversion; owned-high-depth-precision-retained')
            else:
                assert differences == 0, 'expression filter differs from reference'

