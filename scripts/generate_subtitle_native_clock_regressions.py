#!/usr/bin/env python3
"""Authored subtitle overflow cases and a two-frame Y4M companion; no dependencies."""
from pathlib import Path
import json
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/subtitle-native-clocks'
def generate(root):
    root.mkdir(parents=True, exist_ok=True)
    clocks = ['18446744073709551615:00:00', '00:18446744073709551615:00', '00:01:18446744073709551615', '00:00:00.9999999999999999999']
    for i, clock in enumerate(clocks):
        (root / f'overflow-{i}.srt').write_text(f'1\n{clock} --> {clock}\ninvalid clock\n\n2\n00:00:01,5 --> 00:00:02,25\nhealthy cue\n')
    (root / 'companion.y4m').write_bytes(b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\nFRAME\n' + bytes([32]*4+[128]*2) + b'FRAME\n' + bytes([192]*4+[128]*2))
    (root / 'manifest.json').write_text(json.dumps({'provenance':'Authored extreme text clocks, one healthy cue, two flat-color Y4M frames. No private media, external codec or network.','cases':[f'overflow-{i}.srt' for i in range(len(clocks))]},indent=2)+'\n')
if __name__ == '__main__':
    generate(ROOT)
