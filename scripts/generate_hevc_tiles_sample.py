#!/usr/bin/env python3
"""Owned HEVC tile-order reproducer and saved HM acceptance oracle.

Explicit generation only; ordinary tests do not invoke HM or external tools.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-decoder', type=Path, required=True)
args = parser.parse_args()
fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
config_text = '''Profile : main-RExt
Level : 2
IntraPeriod : -1
DecodingRefreshType : 2
GOPSize : 1
Frame1 : B 1 0 0.0 0.0 0 0 1.0 0 0 0 2 2 -1 -2 0
MaxCUWidth : 32
MaxCUHeight : 32
MaxPartitionDepth : 3
QuadtreeTULog2MaxSize : 4
QuadtreeTULog2MinSize : 2
QuadtreeTUMaxDepthInter : 2
QuadtreeTUMaxDepthIntra : 2
QP : 24
SAO : 0
LoopFilterDisable : 1
MaxCUChromaQpAdjustmentDepth : 0
'''
variants = [('two-columns', -1, 3, False, 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\nLFCrossTileBoundaryFlag : 0\n', 8)]
for name, depth, frames, filters, extra, bits in variants:
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-tiles-') as directory:
        tmp = Path(directory)
        config = tmp / 'owned.cfg'
        config.write_text(config_text.replace('MaxCUChromaQpAdjustmentDepth : 0', f'MaxCUChromaQpAdjustmentDepth : {depth}').replace('SAO : 0', f'SAO : {int(filters)}').replace('LoopFilterDisable : 1', f'LoopFilterDisable : {int(not filters)}') + extra)
        source = tmp / 'source.yuv'
        raw = bytearray()
        for frame in range(frames):
            for plane, side in enumerate((64, 32, 32)):
                for y in range(side):
                    for x in range(side):
                        tile = ((x + frame * 2) // 8 + y // 8) % 2
                        raw.append(24 + plane * 19 + (x * 3 + y * 5 + frame * 7) % 112 + tile * 64)
        source.write_bytes(raw)
        stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
        subprocess.run([str(args.hm_encoder.resolve()), '-c', str(config), '-i', str(source),
                        '-b', str(stream), '-o', str(recon), '-wdt', '64', '-hgt', '64', '-fr', '25',
                        '-f', str(frames), '--InputBitDepth=8', f'--InternalBitDepth={bits}', '--InputChromaFormat=420'], check=True)
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                                 f'--OutputBitDepth={bits}', f'--OutputBitDepthC={bits}', '--SEIDecodedPictureHash=0'],
                                check=True, capture_output=True, text=True)
        print(result.stdout)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        pixels = oracle.read_bytes()
        assert len(pixels) == frames * 64 * 64 * 3 // 2 * (1 if bits == 8 else 2)
        assert pixels == recon.read_bytes()
        (fixtures / f'hevc-tiles-{name}-rext{bits}.mp4').write_bytes(mux(stream.read_bytes(), bits))
        (fixtures / f'hevc-tiles-{name}-rext{bits}.yuv').write_bytes(pixels)
