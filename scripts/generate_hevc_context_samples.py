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
args = parser.parse_args()
fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


def ff(*args):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', *map(str, args)], check=True)


with tempfile.TemporaryDirectory(prefix='fvid-hevc-context-') as directory:
    tmp = Path(directory)
    for depth in (8, 10):
        pix = 'yuv420p' if depth == 8 else 'yuv420p10le'
        source = tmp / 'source.yuv'
        ff('-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=25', '-frames:v', 3,
           '-pix_fmt', pix, '-f', 'rawvideo', source)
        for mode in ('skip', 'bypass'):
            for context in (False, True):
                stem = f'hevc-rext-context-{depth}-{mode}-' + ('enabled' if context else 'disabled')
                stream, recon = tmp / 'stream.hevc', tmp / 'recon.yuv'
                options = [str(args.hm_encoder), '-c', str(args.hm_config), '-i', str(source),
                           '-b', str(stream), '-o', str(recon), '-wdt', '64', '-hgt', '64',
                           '-fr', '25', '-f', '3', f'--InputBitDepth={depth}',
                           f'--InternalBitDepth={depth}', '--InputChromaFormat=420',
                           '--MaxCUWidth=32', '--MaxCUHeight=32', '--MaxPartitionDepth=3',
                           '--QuadtreeTULog2MaxSize=2', '--TransformSkip=1',
                           '--TransformSkipLog2MaxSize=2', '--ImplicitResidualDPCM=0',
                           '--ExplicitResidualDPCM=0', '--ResidualRotation=0',
                           '--GolombRiceParameterAdaptation=0', '--HighPrecisionPredictionWeighting=0',
                           '--CrossComponentPrediction=0', '--SAO=0', '--LoopFilterDisable=1',
                           '--QP=24', f'--SingleSignificanceMapContext={int(context)}',
                           f'--TransquantBypassEnable={int(mode == "bypass")}',
                           f'--CUTransquantBypassFlagForce={int(mode == "bypass")}']
                subprocess.run(options, check=True)
                ff('-r', 25, '-i', stream, '-c:v', 'copy', '-tag:v', 'hvc1', fixtures / (stem + '.mp4'))
                ff_output = tmp / 'ff.yuv'
                ff('-i', stream, '-pix_fmt', pix, '-f', 'rawvideo', ff_output)
                hm_output = tmp / 'hm.yuv'
                subprocess.run([str(args.hm_decoder), '-b', str(stream), '-o', str(hm_output),
                                f'--OutputBitDepth={depth}', f'--OutputBitDepthC={depth}'], check=True)
                oracle = hm_output.read_bytes()
                assert oracle == recon.read_bytes() == ff_output.read_bytes()
                assert len(oracle) == 3 * 64 * 64 * 3 // 2 * (1 if depth == 8 else 2)
                (fixtures / (stem + '.yuv')).write_bytes(oracle)
