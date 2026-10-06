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
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
tree = ast.parse((root / 'scripts/generate_hevc_tiles_sample.py').read_text())
config = next(ast.literal_eval(node.value) for node in tree.body
              if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'config_text' for t in node.targets))
config = config.replace('Profile : main-RExt', 'Profile : high-throughput-RExt')
config += 'MaxBitDepthConstraint : 14\nMaxChromaFormatConstraint : 444\nWaveFrontSynchro : 1\nAlignCABACBeforeBypass : 1\nExtendedPrecision : 1\n'
with tempfile.TemporaryDirectory(prefix='fvid-hevc-alignment-') as directory:
    tmp = Path(directory)
    cfg, source = tmp / 'owned.cfg', tmp / 'owned.yuv'
    cfg.write_text(config)
    source.write_bytes(bytes(24 + plane * 19 + (x * 3 + y * 5 + frame * 7) % 112
                             + (((x + frame * 2) // 8 + y // 8) % 2) * 64
                             for frame in range(3) for plane in range(3)
                             for y in range(64) for x in range(64)))
    stream, recon, oracle = tmp / 'active.hevc', tmp / 'recon.yuv', tmp / 'oracle.yuv'
    subprocess.run([str(args.hm_encoder.resolve()), '-c', str(cfg), '-i', str(source),
                    '-b', str(stream), '-o', str(recon), '-wdt', '64', '-hgt', '64',
                    '-fr', '25', '-f', '3', '--InputBitDepth=8', '--InternalBitDepth=12',
                    '--InputChromaFormat=444'], check=True)
    result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(oracle),
                             '--OutputBitDepth=12', '--OutputBitDepthC=12', '--SEIDecodedPictureHash=0'],
                            check=True, capture_output=True, text=True)
    assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
    assert oracle.read_bytes() == recon.read_bytes()
    assert len(oracle.read_bytes()) == 3 * 64 * 64 * 3 * 2
    fixtures = root / 'tests/fixtures/playback-errors'
    (fixtures / 'hevc-cabac-alignment-444-rext12.mp4').write_bytes(
        mux(stream.read_bytes(), 12, chroma_format=3))
    (fixtures / 'hevc-cabac-alignment-444-rext12.yuv').write_bytes(oracle.read_bytes())
