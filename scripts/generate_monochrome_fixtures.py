#!/usr/bin/env python3
"""Write chroma-selective grayscale regressions without media dependencies."""
from pathlib import Path
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8,12,16):
    maximum=(1<<depth)-1
    frames=[]
    for index in range(3):
        samples=[(maximum*n//8+index)%(maximum+1) for n in range(9)]
        samples += [maximum//8,maximum//2,7*maximum//8,maximum]
        samples += [7*maximum//8,maximum//2,maximum//8,0]
        body=bytes(samples) if depth==8 else b''.join(n.to_bytes(2,'little') for n in samples)
        frames.append(b'FRAME\n'+body)
    layout='420' if depth==8 else f'420p{depth}'
    (root/f'monochrome-grid-{depth}.y4m').write_bytes(f'YUV4MPEG2 W3 H3 F2:1 Ip A1:1 C{layout}\n'.encode()+b''.join(frames))
