#!/usr/bin/env python3
"""Generate synthetic HEVC skip/bypass significance-context acceptance fixtures."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import random
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-decoder', type=Path, required=True)
parser.add_argument('--hm-config', type=Path, required=True)
parser.add_argument('--rdpcm', action='store_true', help='generate implicit RDPCM instead of context fixtures')
parser.add_argument('--explicit', action='store_true', help='generate explicit RDPCM in low-delay P pictures')
parser.add_argument('--large-skip', type=int, choices=(3, 4, 5), help='maximum skip block log2 size')
parser.add_argument('--high-precision', action='store_true', help='high precision weighted prediction')
parser.add_argument('--depth', type=int, choices=(8, 10, 12), help='generate one bit depth')
parser.add_argument('--filters', action='store_true', help='enable SAO and deblocking')
parser.add_argument('--qp', type=int, default=24, choices=range(-48, 52))
parser.add_argument('--mode', choices=('skip', 'bypass'))
parser.add_argument('--sao-scale', type=int, choices=(0, 1, 2), default=0)
parser.add_argument('--mixed-bypass', action='store_true', help='allow CU-by-CU bypass decisions')
parser.add_argument('--slice-ctus', type=int, choices=(1, 2), help='independent slices containing this many CTUs')
parser.add_argument('--rice', action='store_true', help='persistent Rice adaptation')
args = parser.parse_args()
if sum((args.rdpcm, args.explicit, args.large_skip is not None, args.high_precision, args.rice)) > 1:
    parser.error('--rdpcm, --explicit, --large-skip and --high-precision and --rice are mutually exclusive')
fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


with tempfile.TemporaryDirectory(prefix='fvid-hevc-context-') as directory:
    tmp = Path(directory)
    config = args.hm_config
    if args.explicit or args.high_precision:
        config = tmp / 'lowdelay.cfg'
        config.write_text(args.hm_config.read_text() + '\nIntraPeriod : -1\nGOPSize : 1\n'
                          'DecodingRefreshType : 2\n'
                          'Frame1 : P 1 0 0.0 0.0 0 0 1.0 0 0 0 1 1 -1 0\n')
    for depth in ((args.depth,) if args.depth else (8, 10)):
        source = tmp / 'source.yuv'
        # Deterministic owned source: moving edges, gradients and textured tiles.
        # No external media or codec parameters enter the synthetic fixture.
        raw = bytearray()
        scale = 1 << (depth - 8)
        for frame in range(3):
            for plane, side in enumerate((64, 32, 32)):
                for y in range(side):
                    for x in range(side):
                        tile = ((x + frame * 3) // 8 + y // 8) % 2
                        value = (24 + plane * 19 + (x * 3 + y * 5 + frame * 17) % 112 + tile * 64) * scale
                        raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2, 'little'))
        source.write_bytes(raw)
        if args.high_precision:
            raw = bytearray()
            scale = 1 << (depth - 8)
            for frame in range(3):
                for plane, side in enumerate((64, 32, 32)):
                    for y in range(side):
                        for x in range(side):
                            value = (48 + plane * 12 + (x + y) % 48 + 40 * frame) * scale + frame
                            raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2, 'little'))
            source.write_bytes(raw)
        if args.mixed_bypass:
            rng = random.Random(20261002)
            raw = bytearray()
            for frame in range(3):
                for side in (64, 32, 32):
                    for y in range(side):
                        for x in range(side):
                            value = (80 << (depth - 8)) if x < side // 2 else rng.randrange(1 << depth)
                            raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2, 'little'))
            source.write_bytes(raw)
        for mode in ((args.mode,) if args.mode else (('skip',) if args.large_skip or args.high_precision else ('skip', 'bypass'))):
            for context in (False, True):
                tool = 'persistent-rice' if args.rice else 'high-precision' if args.high_precision else f'skip{1 << args.large_skip}' if args.large_skip else ('explicit-rdpcm' if args.explicit else ('rdpcm' if args.rdpcm else 'context'))
                stem = f'hevc-rext-{tool}-{depth}-{mode}-' + ('enabled' if context else 'disabled')
                if args.mixed_bypass:
                    stem += '-mixed'
                if args.filters:
                    stem += '-filters'
                if args.sao_scale:
                    stem += f'-sao{args.sao_scale}'
                if args.qp != 24:
                    stem += f'-qp{args.qp}'
                if args.slice_ctus:
                    stem += f'-slices{args.slice_ctus}'
                stream, recon = tmp / 'stream.hevc', tmp / 'recon.yuv'
                options = [str(args.hm_encoder), '-c', str(config), '-i', str(source),
                           '-b', str(stream), '-o', str(recon), '-wdt', '64', '-hgt', '64',
                           '-fr', '25', '-f', '3', f'--InputBitDepth={depth}',
                           f'--InternalBitDepth={depth}', '--InputChromaFormat=420',
                           '--MaxCUWidth=32', '--MaxCUHeight=32', '--MaxPartitionDepth=3',
                           f'--QuadtreeTULog2MaxSize={args.large_skip or 2}', '--TransformSkip=1',
                           f'--TransformSkipLog2MaxSize={args.large_skip if args.large_skip and context else 2}', f'--ImplicitResidualDPCM={int(context and args.rdpcm)}',
                           f'--ExplicitResidualDPCM={int(context and args.explicit)}', '--ResidualRotation=0',
                           f'--GolombRiceParameterAdaptation={int(context and args.rice)}', f'--HighPrecisionPredictionWeighting={int(context and args.high_precision)}',
                           '--CrossComponentPrediction=0', f'--SAO={int(args.filters)}', f'--LoopFilterDisable={int(not args.filters)}',
                           f'--QP={args.qp}', f'--SingleSignificanceMapContext={int(context and not args.rdpcm and not args.explicit and not args.large_skip and not args.high_precision and not args.rice)}',
                           f'--TransquantBypassEnable={int(mode == "bypass")}',
                           f'--CUTransquantBypassFlagForce={int(mode == "bypass" and not args.mixed_bypass)}']
                if args.slice_ctus:
                    options.extend(['--SliceMode=1', f'--SliceArgument={args.slice_ctus}'])
                if args.sao_scale:
                    options.extend([f'--SaoLumaOffsetBitShift={args.sao_scale}', f'--SaoChromaOffsetBitShift={args.sao_scale}'])
                if args.high_precision:
                    options.extend(['--WeightedPredP=1'])
                if args.large_skip:
                    options.append('--ScalingList=1')
                subprocess.run(options, check=True)
                muxed = tmp / 'muxed.mp4'
                muxed.write_bytes(mux(stream.read_bytes(), depth))
                hm_output = tmp / 'hm.yuv'
                subprocess.run([str(args.hm_decoder), '-b', str(stream), '-o', str(hm_output),
                                f'--OutputBitDepth={depth}', f'--OutputBitDepthC={depth}'], check=True)
                oracle = hm_output.read_bytes()
                assert oracle == recon.read_bytes(), 'HM reconstruction and decoder differ'
                if mode == 'bypass' and not args.mixed_bypass and not args.filters:
                    assert oracle == source.read_bytes(), 'forced bypass is not lossless'
                assert len(oracle) == 3 * 64 * 64 * 3 // 2 * (1 if depth == 8 else 2)
                (fixtures / (stem + '.mp4')).write_bytes(muxed.read_bytes())
                (fixtures / (stem + '.yuv')).write_bytes(oracle)
