#!/usr/bin/env python3
"""Regenerate owned long-term-reference HEVC and its independent HM pixel oracle.

Explicit developer command only. Ordinary tests read committed MP4/YUV bytes;
they do not run this generator, HM, FFmpeg, or network tools.
"""
from pathlib import Path
import argparse
import os
import subprocess
import tempfile
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-decoder', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
fixtures = root / 'tests/fixtures/playback-errors'
for mode, name in [('explicit', 'hevc-long-term-rext8'),
                   ('lsb', 'hevc-long-term-lsb-rext8'),
                   ('mixed', 'hevc-long-term-mixed-rext8')]:
    with tempfile.TemporaryDirectory(prefix='fvid-hevc-long-term-') as directory:
        stream = Path(directory) / 'long-term.hevc'
        pixels = Path(directory) / 'long-term.yuv'
        environment = dict(os.environ, FVID_HEVC_LONG_TERM_STREAM=str(stream), FVID_HEVC_LONG_TERM_MODE=mode)
        subprocess.run(['cargo', 'test', '--locked', '--offline', '--manifest-path',
                        'crates/fvid-codecs/Cargo.toml', '--lib', 'generate_long_term_stream',
                        '--', '--ignored'], cwd=root, env=environment, check=True)
        reference = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(pixels),
                        '--OutputBitDepth=8', '--OutputBitDepthC=8',
                        '--SEIDecodedPictureHash=0'], check=True, capture_output=True, text=True)
        print(reference.stdout)
        assert 'inserting lost poc' not in (reference.stdout + reference.stderr).lower(), 'HM concealed a missing reference'
        oracle = pixels.read_bytes()
        assert len(oracle) == 3 * 64 * 64 * 3 // 2
        (fixtures / f'{name}.mp4').write_bytes(mux(stream.read_bytes(), 8))
        (fixtures / f'{name}.yuv').write_bytes(oracle)
        print(f'Generated {name}: three pictures and {len(oracle)} oracle bytes')
