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
parser.add_argument('--extended-precision-only', action='store_true',
                    help='Requires HM built with HIGH_BITDEPTH=ON; generates staged acceptance streams')
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
tile_columns = 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\n'
variants = [
    ('two-columns', -1, 3, False, tile_columns + 'LFCrossTileBoundaryFlag : 0\n', 8, 64, 64),
    ('filtered', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 0\n', 8, 64, 64),
    ('cross-filtered', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 1\n', 8, 64, 64),
    ('asymmetric', -1, 3, True, 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 1\nTileUniformSpacing : 0\nTileColumnWidthArray : 1\nTileRowHeightArray : 2\nLFCrossTileBoundaryFlag : 0\n', 8, 96, 96),
    ('slices', -1, 3, False, tile_columns + 'LFCrossTileBoundaryFlag : 0\nSliceMode : 1\nSliceArgument : 1\n', 8, 64, 64),
    ('dependent', -1, 3, False, tile_columns + 'LFCrossTileBoundaryFlag : 0\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 8, 64, 64),
    ('slices-filtered', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 0\nLFCrossSliceBoundaryFlag : 0\nSliceMode : 1\nSliceArgument : 1\n', 8, 64, 64),
    ('dependent-filtered', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 0\nLFCrossSliceBoundaryFlag : 0\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 8, 64, 64),
    ('cross-segments', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 1\nLFCrossSliceBoundaryFlag : 1\nSliceMode : 1\nSliceArgument : 1\n', 8, 64, 64),
    ('mixed-segments', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 1\nLFCrossSliceBoundaryFlag : 0\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 8, 64, 64),
    ('spanning-segments', -1, 3, True, 'NumTileColumnsMinus1 : 2\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\nLFCrossTileBoundaryFlag : 0\nSliceSegmentMode : 3\nSliceSegmentArgument : 2\n', 8, 96, 64),
    ('mixed-segments-high10', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 1\nLFCrossSliceBoundaryFlag : 0\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 10, 64, 64),
    ('mixed-segments-high12', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 1\nLFCrossSliceBoundaryFlag : 0\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 12, 64, 64),
    ('high10', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 0\n', 10, 64, 64),
    ('high12', -1, 3, True, tile_columns + 'LFCrossTileBoundaryFlag : 0\n', 12, 64, 64),
]
if args.extended_precision_only:
    variants = [(f'extended-precision-high{bits}', -1, 3, True,
                 tile_columns + 'LFCrossTileBoundaryFlag : 0\nExtendedPrecision : 1\n',
                 bits, 64, 64) for bits in [8, 12]]
for name, depth, frames, filters, extra, bits, width, height in variants:
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-tiles-') as directory:
        tmp = Path(directory)
        config = tmp / 'owned.cfg'
        config.write_text(config_text.replace('MaxCUChromaQpAdjustmentDepth : 0', f'MaxCUChromaQpAdjustmentDepth : {depth}').replace('SAO : 0', f'SAO : {int(filters)}').replace('LoopFilterDisable : 1', f'LoopFilterDisable : {int(not filters)}') + extra)
        source = tmp / 'source.yuv'
        raw = bytearray()
        for frame in range(frames):
            for plane in range(3):
                scale = 1 if plane == 0 else 2
                for y in range(height // scale):
                    for x in range(width // scale):
                        tile = ((x + frame * 2) // 8 + y // 8) % 2
                        raw.append(24 + plane * 19 + (x * 3 + y * 5 + frame * 7) % 112 + tile * 64)
        source.write_bytes(raw)
        stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
        subprocess.run([str(args.hm_encoder.resolve()), '-c', str(config), '-i', str(source),
                        '-b', str(stream), '-o', str(recon), '-wdt', str(width), '-hgt', str(height), '-fr', '25',
                        '-f', str(frames), '--InputBitDepth=8', f'--InternalBitDepth={bits}', '--InputChromaFormat=420'], check=True)
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                                 f'--OutputBitDepth={bits}', f'--OutputBitDepthC={bits}', '--SEIDecodedPictureHash=0'],
                                check=True, capture_output=True, text=True)
        print(result.stdout)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        pixels = oracle.read_bytes()
        assert len(pixels) == frames * width * height * 3 // 2 * (1 if bits == 8 else 2)
        assert pixels == recon.read_bytes()
        (fixtures / f'hevc-tiles-{name}-rext{bits}.mp4').write_bytes(mux(stream.read_bytes(), bits, width, height))
        (fixtures / f'hevc-tiles-{name}-rext{bits}.yuv').write_bytes(pixels)
