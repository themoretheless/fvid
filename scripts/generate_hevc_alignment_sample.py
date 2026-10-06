#!/usr/bin/env python3
"""Generate owned high-throughput HEVC CABAC alignment regression with HM.
Explicit generation only; normal tests consume committed bytes.
"""
import argparse
import ast
from pathlib import Path
import subprocess
import tempfile
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-decoder', type=Path, required=True)
parser.add_argument('--full-chroma-suite', action='store_true', help='Also generate ordinary 4:4:4 filtering and segment streams')
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
tree = ast.parse((root / 'scripts/generate_hevc_tiles_sample.py').read_text())
config = next(ast.literal_eval(node.value) for node in tree.body
              if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'config_text' for t in node.targets))
base_config = config
config = config.replace('Profile : main-RExt', 'Profile : high-throughput-RExt')
config += 'MaxBitDepthConstraint : 14\nMaxChromaFormatConstraint : 444\nWaveFrontSynchro : 1\nAlignCABACBeforeBypass : 1\nExtendedPrecision : 1\n'
variants = [('hevc-cabac-alignment-444-rext12', config, 12)]
if args.full_chroma_suite:
    filtered = base_config.replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    variants += [(f'hevc-full-chroma-filtered-rext{bits}', filtered, bits) for bits in [8,10,12]]
    tiles = 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\n'
    variants += [
        ('hevc-full-chroma-high-qp-rext12', filtered.replace('QP : 24', 'QP : 40'), 12),
        ('hevc-full-chroma-wpp-rext12', filtered + 'WaveFrontSynchro : 1\n', 12),
        ('hevc-full-chroma-mixed-tiles-rext12', filtered + tiles + 'LFCrossTileBoundaryFlag : 1\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 12),
    ]
variants = [(stem, cfg, bits, 64, 64) for stem, cfg, bits in variants]
if args.full_chroma_suite:
    variants.append(('hevc-full-chroma-parallel-rext12', filtered + 'WaveFrontSynchro : 1\n', 12, 128, 96))
for stem, config, depth, width, height in variants:
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-alignment-') as directory:
        tmp = Path(directory)
        cfg, source = tmp / 'owned.cfg', tmp / 'owned.yuv'
        cfg.write_text(config)
        source.write_bytes(bytes(24 + plane * 19 + (x * 3 + y * 5 + frame * 7) % 112
                                 + (((x + frame * 2) // 8 + y // 8) % 2) * 64
                                 for frame in range(3) for plane in range(3)
                                 for y in range(height) for x in range(width)))
        stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
        subprocess.run([str(args.hm_encoder.resolve()), '-c', str(cfg), '-i', str(source),
                        '-b', str(stream), '-o', str(recon), '-wdt', str(width), '-hgt', str(height),
                        '-fr', '25', '-f', '3', '--InputBitDepth=8', f'--InternalBitDepth={depth}',
                        '--InputChromaFormat=444'], check=True)
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                                 f'--OutputBitDepth={depth}', f'--OutputBitDepthC={depth}', '--SEIDecodedPictureHash=0'],
                                check=True, capture_output=True, text=True)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        assert oracle.read_bytes() == recon.read_bytes()
        assert len(oracle.read_bytes()) == 3 * width * height * 3 * (1 if depth == 8 else 2)
        fixtures = root / 'tests/fixtures/playback-errors'
        (fixtures / f'{stem}.mp4').write_bytes(
            mux(stream.read_bytes(), depth, width, height, chroma_format=3))
        (fixtures / f'{stem}.yuv').write_bytes(oracle.read_bytes())
