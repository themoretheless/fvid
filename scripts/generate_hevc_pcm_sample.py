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
parser.add_argument('--monochrome-only',action='store_true',help='Generate monochrome PCM corpus')
parser.add_argument('--chroma-format',type=int,choices=[420,422,444],default=420)
parser.add_argument('--deep-depth-only',action='store_true',help='Generate 14/16-bit PCM high-throughput intra corpus')
args = parser.parse_args()
if args.monochrome_only and args.chroma_format != 420:
    parser.error('--monochrome-only cannot be combined with a chroma format')
chroma_format = 0 if args.monochrome_only else {420:1,422:2,444:3}[args.chroma_format]
prefix = 'hevc-pcm-mono' if args.monochrome_only else ('hevc-pcm' if chroma_format == 1 else f'hevc-pcm-{args.chroma_format}')
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
    ('small8', 64, 64, 8, True, False, 'PCMLog2MinSize : 3\nPCMLog2MaxSize : 3\n'),
    ('small16', 64, 64, 8, True, False, 'PCMLog2MinSize : 4\nPCMLog2MaxSize : 4\n'),
    ('mixed', 64, 64, 8, True, True, ''),
    ('filtered', 64, 64, 8, True, False, 'PCMFilterDisableFlag : 0\n'),
    ('parallel', 128, 96, 8, True, True, ''),
    ('high10', 64, 64, 10, True, False, 'PCMInputBitDepthFlag : 1\n'),
    ('high12', 64, 64, 12, True, False, 'PCMInputBitDepthFlag : 1\n'),
    ('full10', 64, 64, 10, True, False, 'PCMInputBitDepthFlag : 0\n'),
    ('full12', 64, 64, 12, True, False, 'PCMInputBitDepthFlag : 0\n'),
    ('wpp', 64, 64, 8, True, True, 'WaveFrontSynchro : 1\n'),
    ('reference', 64, 64, 8, True, False, ''),
    ('reference-wpp', 64, 64, 8, True, False, 'WaveFrontSynchro : 1\n'),
    ('dependent', 64, 64, 8, True, True, 'SliceSegmentMode : 1\nSliceSegmentArgument : 1\n'),
]
if args.deep_depth_only:
    if args.monochrome_only or args.chroma_format != 444:
        parser.error('--deep-depth-only requires --chroma-format 444')
    config_text = '\n'.join(line for line in config_text.splitlines() if not line.startswith('Frame1 :'))+'\n'
    config_text = config_text.replace('Profile : main-RExt','Profile : high-throughput-RExt').replace('IntraPeriod : -1','IntraPeriod : 1').replace('DecodingRefreshType : 2','DecodingRefreshType : 0')
    config_text += 'MaxBitDepthConstraint : 16\nMaxChromaFormatConstraint : 444\nIntraConstraintFlag : 1\nExtendedPrecision : 1\nAlignCABACBeforeBypass : 1\n'
    variants = [(name,width,height,bits,True,mixed,opts)
                for bits in [14,16] for name,width,height,mixed,opts in [
                    ('deep-input8',64,64,False,'PCMInputBitDepthFlag : 1\n'),
                    ('deep-full',64,64,False,'PCMInputBitDepthFlag : 0\n'),
                    ('deep-mixed',64,64,True,'PCMInputBitDepthFlag : 0\nPCMFilterDisableFlag : 0\n'),
                    ('deep-dependent',64,64,True,'PCMInputBitDepthFlag : 0\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n'),
                    ('deep-parallel',128,96,True,'PCMInputBitDepthFlag : 0\nWaveFrontSynchro : 1\n')]]
for name, width, height, bits, filters, mixed, options in variants:
    frames = 3 if name.startswith('reference') or args.deep_depth_only else 1
    input_bits = bits if args.deep_depth_only and name != 'deep-input8' else 8
    extra = pcm_config + options
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-pcm-') as directory:
        tmp = Path(directory)
        config = tmp / 'owned.cfg'
        active_config = config_text.replace('QP : 0',f'QP : {-6*(bits-8)}') if args.deep_depth_only else config_text
        config.write_text(active_config.replace('SAO : 0', f'SAO : {int(filters)}').replace('LoopFilterDisable : 1', f'LoopFilterDisable : {int(not filters)}') + extra)
        source = tmp / 'source.yuv'
        raw = bytearray()
        state = 0x12345678
        for plane in range(1 if chroma_format == 0 else 3):
            scale_x = 1 if plane == 0 or chroma_format == 3 else 2
            scale_y = 2 if plane != 0 and chroma_format == 1 else 1
            for y in range(height // scale_y):
                for x in range(width // scale_x):
                    state = (1664525 * state + 1013904223) & 0xffffffff
                    noise = not mixed or ((x * scale_x // 32 + y * scale_y // 32) % 2 == 0)
                    value = (state >> (32-input_bits)) if noise else ((32+plane*40) << (input_bits-8))
                    if input_bits==8: raw.append(value)
                    else: raw.extend(value.to_bytes(2,'little'))
        source.write_bytes(raw * frames)
        stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
        subprocess.run([str(args.hm_encoder.resolve()), '-c', str(config), '-i', str(source),
                        '-b', str(stream), '-o', str(recon), '-wdt', str(width), '-hgt', str(height), '-fr', '25',
                        '-f', str(frames), f'--InputBitDepth={input_bits}', f'--InternalBitDepth={bits}', f'--InputChromaFormat={ {0:400,1:420,2:422,3:444}[chroma_format] }'], check=True)
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                                 f'--OutputBitDepth={bits}', f'--OutputBitDepthC={bits}', '--SEIDecodedPictureHash=0'],
                                check=True, capture_output=True, text=True)
        print(result.stdout)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        pixels = oracle.read_bytes()
        assert len(pixels) == frames * width * height * {0:2,1:3,2:4,3:6}[chroma_format] // 2 * (1 if bits == 8 else 2)
        assert pixels == recon.read_bytes()
        (fixtures / f'{prefix}-{name}-rext{bits}.mp4').write_bytes(mux(stream.read_bytes(), bits, width, height,chroma_format=chroma_format))
        (fixtures / f'{prefix}-{name}-rext{bits}.yuv').write_bytes(pixels)
