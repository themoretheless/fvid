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
parser.add_argument('--chroma422-only', action='store_true', help='Generate 4:2:2 filtering, WPP and segment acceptance streams')
parser.add_argument('--cross-component-only', action='store_true', help='Generate staged 4:4:4 cross-component prediction streams')
parser.add_argument('--monochrome-only', action='store_true', help='Generate monochrome acceptance streams')
parser.add_argument('--mixed-depth-only',action='store_true',help='Generate mixed luma/chroma depth streams')
parser.add_argument('--deep-depth-only',action='store_true',help='Generate 14/16-bit high-throughput RExt streams')
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
chroma_format = 3
if args.chroma422_only:
    chroma_format = 2
    filtered = base_config.replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    variants = [(f'hevc-chroma422-filtered-rext{bits}', filtered, bits, 64, 64) for bits in [8,10,12]]
    tiles = 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\n'
    variants += [
        ('hevc-chroma422-wpp-rext12', filtered + 'WaveFrontSynchro : 1\n', 12, 64, 64),
        ('hevc-chroma422-mixed-tiles-rext12', filtered + tiles + 'LFCrossTileBoundaryFlag : 1\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 12, 64, 64),
        ('hevc-chroma422-parallel-rext12', filtered + 'WaveFrontSynchro : 1\n', 12, 128, 96),
    ]
if args.cross_component_only:
    chroma_format = 3
    filtered = base_config.replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    variants = [(f'hevc-cross-component-rext{bits}', filtered + 'CrossComponentPrediction : 1\nReconBasedCrossCPredictionEstimate : 1\n', bits, 64, 64) for bits in [8,10,12]]
    variants.append(('hevc-cross-component-parallel-rext12', filtered + 'CrossComponentPrediction : 1\nReconBasedCrossCPredictionEstimate : 1\nWaveFrontSynchro : 1\n', 12, 128, 96))
if args.monochrome_only:
    chroma_format = 0
    filtered = base_config.replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    filtered = filtered.replace('MaxCUChromaQpAdjustmentDepth : 0', 'MaxCUChromaQpAdjustmentDepth : -1')
    variants = [(f'hevc-monochrome-filtered-rext{bits}', filtered, bits, 64, 64) for bits in [8,10,12]]
    tiles = 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\n'
    variants += [
        ('hevc-monochrome-wpp-rext12', filtered + 'WaveFrontSynchro : 1\n', 12, 64, 64),
        ('hevc-monochrome-mixed-tiles-rext12', filtered + tiles + 'LFCrossTileBoundaryFlag : 1\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n', 12, 64, 64),
        ('hevc-monochrome-parallel-rext12', filtered + 'WaveFrontSynchro : 1\n', 12, 128, 96),
    ]
if args.mixed_depth_only:
    chroma_format = 3
    filtered = base_config.replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    variants = [(f'hevc-mixed-depth-y{y}-c{c}',filtered,(y,c),64,64) for y,c in [(8,10),(10,8),(12,10)]]
    tiles = 'NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\n'
    for y,c in [(8,10),(10,8),(12,10)]:
        prefix = f'hevc-mixed-depth-y{y}-c{c}'
        variants += [
            (prefix + '-wpp',filtered + 'WaveFrontSynchro : 1\n',(y,c),64,64),
            (prefix + '-mixed-tiles',filtered + tiles + 'LFCrossTileBoundaryFlag : 1\nSliceMode : 1\nSliceArgument : 2\nSliceSegmentMode : 1\nSliceSegmentArgument : 1\n',(y,c),64,64),
            (prefix + '-parallel',filtered + 'WaveFrontSynchro : 1\n',(y,c),128,96),
            (prefix + '-cross',filtered + 'CrossComponentPrediction : 1\nReconBasedCrossCPredictionEstimate : 1\n',(y,c),64,64),
        ]
if args.deep_depth_only:
    chroma_format = 3
    filtered = base_config.replace('SAO : 0','SAO : 1').replace('LoopFilterDisable : 1','LoopFilterDisable : 0').replace('Profile : main-RExt','Profile : high-throughput-RExt')
    filtered = '\n'.join(line for line in filtered.splitlines() if not line.startswith('Frame1 :'))+'\n'
    filtered += 'MaxChromaFormatConstraint : 444\nExtendedPrecision : 1\n'
    variants = [(f'hevc-deep-rext{bits}{suffix}',filtered.replace('IntraPeriod : -1','IntraPeriod : 1').replace('DecodingRefreshType : 2','DecodingRefreshType : 0').replace('Frame1 : B','Frame1 : I')+'MaxBitDepthConstraint : 16\nIntraConstraintFlag : 1\nAlignCABACBeforeBypass : 1\n'+('WaveFrontSynchro : 1\n' if suffix else ''),bits,width,height)
                for bits in [14,16] for suffix,width,height in [('',64,64),('-parallel',128,96)]]
if args.deep_depth_only:
    inter = base_config.replace('SAO : 0','SAO : 1').replace('LoopFilterDisable : 1','LoopFilterDisable : 0').replace('Profile : main-RExt','Profile : high-throughput-RExt')
    inter += 'MaxBitDepthConstraint : 14\nMaxChromaFormatConstraint : 444\nExtendedPrecision : 1\nWaveFrontSynchro : 1\nAlignCABACBeforeBypass : 1\n'
    variants += [('hevc-deep-rext14-inter',inter,14,64,64),('hevc-deep-rext14-inter-parallel',inter,14,128,96)]
for stem, config, depth, width, height in variants:
    depth, chroma_depth = depth if isinstance(depth,tuple) else (depth,depth)
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-alignment-') as directory:
        tmp = Path(directory)
        cfg, source = tmp / 'owned.cfg', tmp / 'owned.yuv'
        cfg.write_text(config)
        source.write_bytes(bytes(24 + plane * 19 + (x * 3 + y * 5 + frame * 7) % 112
                                 + (((x + frame * 2) // 8 + y // 8) % 2) * 64
                                 for frame in range(3) for plane in range(1 if chroma_format == 0 else 3)
                                 for y in range(height) for x in range(width if plane == 0 or chroma_format == 3 else width // 2)))
        stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
        subprocess.run([str(args.hm_encoder.resolve()), '-c', str(cfg), '-i', str(source),
                        '-b', str(stream), '-o', str(recon), '-wdt', str(width), '-hgt', str(height),
                        '-fr', '25', '-f', '3', '--InputBitDepth=8', f'--InternalBitDepth={depth}', f'--InternalBitDepthC={chroma_depth}',
                        f'--InputChromaFormat={ {0:400,2:422,3:444}[chroma_format] }'], check=True)
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                                 f'--OutputBitDepth={depth}', f'--OutputBitDepthC={chroma_depth}', '--SEIDecodedPictureHash=0'],
                                check=True, capture_output=True, text=True)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        assert oracle.read_bytes() == recon.read_bytes()
        # HM serializes all components with the same sample word width.
        samples_per_frame = width * height * {0:1,2:2,3:3}[chroma_format]
        assert len(oracle.read_bytes()) == 3 * samples_per_frame * (1 if max(depth,chroma_depth) == 8 else 2)
        fixtures = root / 'tests/fixtures/playback-errors'
        (fixtures / f'{stem}.mp4').write_bytes(
            mux(stream.read_bytes(), depth, width, height, chroma_format=chroma_format,chroma_depth=chroma_depth))
        (fixtures / f'{stem}.yuv').write_bytes(oracle.read_bytes())
