#!/usr/bin/env python3
"""Synthetic unsharp edge/impulse samples, no external encoder."""
from pathlib import Path
import struct
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for depth in [8,10,16]:
    maximum=(1<<depth)-1
    values=[((x*31+y*47+plane*53)%256)*maximum//255 for plane in range(3) for y in range(8) for x in range(8)]
    values[27]=maximum
    samples=bytes(values) if depth==8 else b''.join(struct.pack('<H',v) for v in values)
    chroma='444' if depth==8 else f'444p{depth}'
    (root/f'unsharp-{depth}.y4m').write_bytes(f'YUV4MPEG2 W8 H8 F30:1 Ip C{chroma}\n'.encode()+b'FRAME\n'+samples)
    impulse=[0]*9+[1<<(depth-1)]*18
    impulse[4]=100<<(depth-8)
    payload=bytes(impulse) if depth==8 else b''.join(struct.pack('<H',v) for v in impulse)
    (root/f'unsharp-impulse-{depth}.y4m').write_bytes(f'YUV4MPEG2 W3 H3 F30:1 Ip C{chroma}\n'.encode()+b'FRAME\n'+payload)
