#!/usr/bin/env python3
"""Synthetic AAC-LC mono silence for concat dispatch; no external tools."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
# SCE, tag 0, global gain 100, long sine window, zero scale-factor bands,
# no prediction/pulse/TNS/gain control, END, alignment padding.
fields = [(0,3),(0,4),(100,8),(0,1),(0,2),(0,1),(0,6),(0,1),(0,1),(0,1),(0,1),(7,3)]
bits = ''.join(format(value, f'0{width}b') for value,width in fields)
bits += '0' * (-len(bits) % 8)
payload = int(bits,2).to_bytes(len(bits)//8,'big')
size = 7 + len(payload)
header = bytes([0xff,0xf1,0x50,0x40 | (size >> 11),(size >> 3)&255,((size&7)<<5)|31,0xfc])
for name in ['adts-concat-a.aac', 'adts-concat-b.aac']:
    (root / name).write_bytes(header + payload)
