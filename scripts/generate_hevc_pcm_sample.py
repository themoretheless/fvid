#!/usr/bin/env python3
"""Owned HEVC PCM reproducer and saved HM acceptance oracle.

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
QP : 0
SAO : 0
LoopFilterDisable : 1
MaxCUChromaQpAdjustmentDepth : -1
'''
pcm_config = 'PCMEnabledFlag : 1\nPCMLog2MinSize : 5\nPCMLog2MaxSize : 5\nPCMFilterDisableFlag : 1\n'
variants = [
    ('active', 64, 64, 8, False, False, ''),
    ('mixed', 64, 64, 8, True, True, ''),
    ('filtered', 64, 64, 8, True, False, 'PCMFilterDisableFlag : 0\n'),
    ('parallel', 128, 96, 8, True, True, ''),
    ('high10', 64, 64, 10, True, False, 'PCMInputBitDepthFlag : 1\n'),
    ('high12', 64, 64, 12, True, False, 'PCMInputBitDepthFlag : 1\n'),
    ('full10', 64, 64, 10, True, False, 'PCMInputBitDepthFlag : 0\n'),
    ('full12', 64, 64, 12, True, False, 'PCMInputBitDepthFlag : 0\n'),
    ('wpp', 64, 64, 8, True, True, 'WaveFrontSynchro : 1\n'),
    ('dependent', 64, 64, 8, True, True, 'SliceSegmentMode : 1\nSliceSegmentArgument : 1\n'),
]
for name, width, height, bits, filters, mixed, options in variants:
    depth, frames = 0, 1
    extra = pcm_config + options
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-chroma-qp-') as directory:
        tmp = Path(directory)
        config = tmp / 'owned.cfg'
        config.write_text(config_text.replace('SAO : 0', f'SAO : {int(filters)}').replace('LoopFilterDisable : 1', f'LoopFilterDisable : {int(not filters)}') + extra)
        source = tmp / 'source.yuv'
        raw = bytearray()
        state = 0x12345678
        for plane in range(3):
            scale = 1 if plane == 0 else 2
            for y in range(height // scale):
                for x in range(width // scale):
                    state = (1664525 * state + 1013904223) & 0xffffffff
                    noise = not mixed or ((x * scale // 32 + y * scale // 32) % 2 == 0)
                    raw.append(state >> 24 if noise else 32 + plane * 40)
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
        (fixtures / f'hevc-pcm-{name}-rext{bits}.mp4').write_bytes(mux(stream.read_bytes(), bits, width, height))
        (fixtures / f'hevc-pcm-{name}-rext{bits}.yuv').write_bytes(pixels)
