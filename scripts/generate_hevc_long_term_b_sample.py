#!/usr/bin/env python3
"""Explicit owned I/B/B/B generation and HM oracle; never run by ordinary tests."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-decoder', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
fixtures = root / 'tests/fixtures/playback-errors'
config_text = '''Profile : main
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
FastMEForGenBLowDelayEnabled : 0
'''
with tempfile.TemporaryDirectory(prefix='fvid-hevc-b-long-term-') as directory:
    tmp = Path(directory)
    config = tmp / 'owned.cfg'
    config.write_text(config_text)
    source = tmp / 'source.yuv'
    raw = bytearray()
    for frame in range(4):
        for plane, side in enumerate((64, 32, 32)):
            for y in range(side):
                for x in range(side):
                    tile = ((x + frame * 3) // 8 + y // 8) % 2
                    raw.append(24 + plane * 19 + (x * 3 + y * 5 + frame * 17) % 112 + tile * 64)
    frame_bytes = 64 * 64 * 3 // 2
    # A blended final picture forces useful bi-prediction across both references.
    raw[3 * frame_bytes:] = bytes((a + b) // 2 for a, b in
                                  zip(raw[frame_bytes:2 * frame_bytes], raw[2 * frame_bytes:3 * frame_bytes]))
    source.write_bytes(raw)
    base, recon = tmp / 'base.hevc', tmp / 'recon.yuv'
    subprocess.run([str(args.hm_encoder.resolve()), '-c', str(config), '-i', str(source),
                    '-b', str(base), '-o', str(recon), '-wdt', '64', '-hgt', '64',
                    '-fr', '25', '-f', '4', '--InputBitDepth=8', '--InternalBitDepth=8',
                    '--InputChromaFormat=420'], check=True)
    def oracle(stream):
        output = tmp / 'oracle.yuv'
        result = subprocess.run([str(args.hm_decoder.resolve()), '-b', str(stream), '-o', str(output),
                                 '--OutputBitDepth=8', '--OutputBitDepthC=8',
                                 '--SEIDecodedPictureHash=0'], capture_output=True, text=True, check=True)
        print(result.stdout)
        assert 'inserting lost poc' not in (result.stdout + result.stderr).lower()
        pixels = output.read_bytes()
        assert len(pixels) == 4 * 64 * 64 * 3 // 2
        return pixels
    pixels = oracle(base)
    assert pixels == recon.read_bytes()
    (fixtures / 'hevc-long-term-b-base-main8.mp4').write_bytes(mux(base.read_bytes(), 8))
    (fixtures / 'hevc-long-term-b-base-main8.yuv').write_bytes(pixels)
    for mode, name in [('b-mixed', 'hevc-long-term-b-mixed-main8'),
                       ('b-mixed-l1', 'hevc-long-term-b-mixed-l1-main8')]:
        mixed = tmp / 'mixed.hevc'
        environment = dict(os.environ, FVID_HEVC_LONG_TERM_MODE=mode, FVID_HEVC_LONG_TERM_STREAM=str(mixed))
        subprocess.run(['cargo', 'test', '--locked', '--offline', '--manifest-path',
                        'crates/fvid-codecs/Cargo.toml', '--lib', 'generate_long_term_stream',
                        '--', '--ignored'], cwd=root, env=environment, check=True)
        pixels = oracle(mixed)
        (fixtures / f'{name}.mp4').write_bytes(mux(mixed.read_bytes(), 8))
        (fixtures / f'{name}.yuv').write_bytes(pixels)
