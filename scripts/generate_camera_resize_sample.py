#!/usr/bin/env python3
"""Synthetic I/P/P camera resize fixture: HM reference encoder, owned MP4 muxer."""
import argparse
from pathlib import Path
import subprocess
import tempfile
from hevc_fixture_mp4 import mux

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-config', type=Path, required=True)
parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/hevc-camera-resize.mp4')
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix='fvid-camera-resize-') as temp:
    directory = Path(temp)
    config = directory / 'encoder.cfg'
    config.write_text(args.hm_config.read_text() + '\nIntraPeriod : -1\nGOPSize : 1\n'
        'DecodingRefreshType : 2\nFrame1 : P 1 0 0.0 0.0 0 0 1.0 0 0 0 1 1 -1 0\n')
    sequences = []
    for width, height in [(128, 128), (96, 64)]:
        source, stream, recon = [directory / f'{width}-{name}' for name in ['source.yuv', 'stream.hevc', 'recon.yuv']]
        raw = bytearray()
        for frame in range(3):
            for plane in range(3):
                w, h = (width, height) if plane == 0 else (width // 2, height // 2)
                raw.extend(24 + (x * 3 + y * 5 + frame * 17 + plane * 29) % 208
                           for y in range(h) for x in range(w))
        source.write_bytes(raw)
        subprocess.run([str(args.hm_encoder), '-c', str(config), '-i', str(source), '-b', str(stream), '-o', str(recon),
            '-wdt', str(width), '-hgt', str(height), '-fr', '30', '-f', '3', '--InputBitDepth=8', '--InternalBitDepth=8',
            '--InputChromaFormat=420', '--MaxCUWidth=32', '--MaxCUHeight=32', '--MaxPartitionDepth=3',
            '--QuadtreeTULog2MaxSize=4', '--SAO=0', '--LoopFilterDisable=1', '--QP=24'], check=True)
        sequences.append(stream.read_bytes())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(mux(b''.join(sequences), 8, 128, 128, 30, inband_parameters=True))
