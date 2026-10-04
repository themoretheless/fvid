#!/usr/bin/env python3
"""Synthetic odd YUV frames proving exposure clips at final output, not RGB workspace."""
from pathlib import Path
import struct
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for depth in (8,12,16):
    scale=1<<(depth-8)
    chroma='420' if depth==8 else f'420p{depth}'
    result=bytearray(f'YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C{chroma}\n'.encode())
    for luma in (235,128,16):
        result.extend(b'FRAME\n')
        samples=[luma*scale]*9+[128*scale]*8
        result.extend(bytes(samples) if depth==8 else b''.join(struct.pack('<H',v) for v in samples))
    (root/f'exposure-headroom-{depth}.y4m').write_bytes(result)
