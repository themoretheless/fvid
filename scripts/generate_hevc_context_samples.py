#!/usr/bin/env python3
"""Generate synthetic HEVC skip/bypass significance-context acceptance fixtures."""
import argparse
from pathlib import Path
import subprocess
import tempfile

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
parser.add_argument('--qp', type=int, default=24, choices=range(0, 52))
parser.add_argument('--mode', choices=('skip', 'bypass'))
args = parser.parse_args()
if sum((args.rdpcm, args.explicit, args.large_skip is not None, args.high_precision)) > 1:
    parser.error('--rdpcm, --explicit, --large-skip and --high-precision are mutually exclusive')
fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


def ff(*args):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', *map(str, args)], check=True)


with tempfile.TemporaryDirectory(prefix='fvid-hevc-context-') as directory:
    tmp = Path(directory)
    config = args.hm_config
    if args.explicit or args.high_precision:
        config = tmp / 'lowdelay.cfg'
        config.write_text(args.hm_config.read_text() + '\nIntraPeriod : -1\nGOPSize : 1\n'
                          'DecodingRefreshType : 2\n'
                          'Frame1 : P 1 0 0.0 0.0 0 0 1.0 0 0 0 1 1 -1 0\n')
    for depth in ((args.depth,) if args.depth else (8, 10)):
        pix = 'yuv420p' if depth == 8 else f'yuv420p{depth}le'
        source = tmp / 'source.yuv'
        ff('-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=25', '-frames:v', 3,
           '-pix_fmt', pix, '-f', 'rawvideo', source)
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
        for mode in ((args.mode,) if args.mode else (('skip',) if args.large_skip or args.high_precision else ('skip', 'bypass'))):
            for context in (False, True):
                tool = 'high-precision' if args.high_precision else f'skip{1 << args.large_skip}' if args.large_skip else ('explicit-rdpcm' if args.explicit else ('rdpcm' if args.rdpcm else 'context'))
                stem = f'hevc-rext-{tool}-{depth}-{mode}-' + ('enabled' if context else 'disabled')
                if args.filters:
                    stem += '-filters'
                if args.qp != 24:
                    stem += f'-qp{args.qp}'
                stream, recon = tmp / 'stream.hevc', tmp / 'recon.yuv'
                options = [str(args.hm_encoder), '-c', str(config), '-i', str(source),
                           '-b', str(stream), '-o', str(recon), '-wdt', '64', '-hgt', '64',
                           '-fr', '25', '-f', '3', f'--InputBitDepth={depth}',
                           f'--InternalBitDepth={depth}', '--InputChromaFormat=420',
                           '--MaxCUWidth=32', '--MaxCUHeight=32', '--MaxPartitionDepth=3',
                           f'--QuadtreeTULog2MaxSize={args.large_skip or 2}', '--TransformSkip=1',
                           f'--TransformSkipLog2MaxSize={args.large_skip if args.large_skip and context else 2}', f'--ImplicitResidualDPCM={int(context and args.rdpcm)}',
                           f'--ExplicitResidualDPCM={int(context and args.explicit)}', '--ResidualRotation=0',
                           '--GolombRiceParameterAdaptation=0', f'--HighPrecisionPredictionWeighting={int(context and args.high_precision)}',
                           '--CrossComponentPrediction=0', f'--SAO={int(args.filters)}', f'--LoopFilterDisable={int(not args.filters)}',
                           f'--QP={args.qp}', f'--SingleSignificanceMapContext={int(context and not args.rdpcm and not args.explicit and not args.large_skip and not args.high_precision)}',
                           f'--TransquantBypassEnable={int(mode == "bypass")}',
                           f'--CUTransquantBypassFlagForce={int(mode == "bypass")}']
                if args.high_precision:
                    options.extend(['--WeightedPredP=1'])
                if args.large_skip:
                    options.append('--ScalingList=1')
                subprocess.run(options, check=True)
                muxed = tmp / 'muxed.mp4'
                ff('-r', 25, '-i', stream, '-c:v', 'copy', '-tag:v', 'hvc1', muxed)
                ff_output = tmp / 'ff.yuv'
                ff('-i', stream, '-pix_fmt', pix, '-f', 'rawvideo', ff_output)
                hm_output = tmp / 'hm.yuv'
                subprocess.run([str(args.hm_decoder), '-b', str(stream), '-o', str(hm_output),
                                f'--OutputBitDepth={depth}', f'--OutputBitDepthC={depth}'], check=True)
                oracle = hm_output.read_bytes()
                assert oracle == recon.read_bytes(), 'HM reconstruction and decoder differ'
                if args.rdpcm and mode == 'bypass' and context:
                    assert oracle == source.read_bytes(), 'forced bypass is not lossless'
                    print(f'{stem}: FFmpeg differs in {sum(a != b for a, b in zip(oracle, ff_output.read_bytes()))} oracle bytes')
                elif (args.high_precision and context and depth == 10) or (depth == 12 and args.filters and args.qp == 51):
                    print(f'{stem}: FFmpeg differs in {sum(a != b for a, b in zip(oracle, ff_output.read_bytes()))} oracle bytes')
                else:
                    assert oracle == ff_output.read_bytes(), 'HM and FFmpeg differ'
                assert len(oracle) == 3 * 64 * 64 * 3 // 2 * (1 if depth == 8 else 2)
                (fixtures / (stem + '.mp4')).write_bytes(muxed.read_bytes())
                (fixtures / (stem + '.yuv')).write_bytes(oracle)
