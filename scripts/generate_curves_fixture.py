#!/usr/bin/env python3
"""Synthetic grayscale playback, RGB plateau ramp and ACV controls."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
for depth in (8,10):
    data = bytearray(f'YUV4MPEG2 W4 H4 F25:1 Ip C444{("p10" if depth==10 else "")}\n'.encode())
    for luma in (16,235,16,235):
        data += b'FRAME\n'
        for value,count in ((luma,16),(128,32)):
            value <<= depth-8
            data += (bytes([value]) if depth==8 else struct.pack('<H',value))*count
    (root / f'curves-gray-{depth}.y4m').write_bytes(data)
for depth in (8,16):
    data = bytearray()
    for i in range(256):
        for v in (i,255-i,(i*37)%256):
            data += bytes([v]) if depth==8 else struct.pack('<H',v*257)
    (root / f'curves-plateau.rgb{depth*3}').write_bytes(data)
data = bytearray(struct.pack('>HH',4,4))
for points in (((0,255),(255,0)),((0,0),(255,255)),((0,0),(255,255)),((0,0),(255,255))):
    data += struct.pack('>H',len(points))
    for x,y in points:
        data += struct.pack('>HH',y,x)
(root / 'curves-negative.acv').write_bytes(data)


# Reuse only the first fragment of the hash-pinned repository synthetic AVC seed.
# Fragment-relative offsets are preserved; no codec tool or network is needed.
import hashlib
project = root.parents[2]
seed = (project / 'tests/fixtures/video.mp4').read_bytes()
assert hashlib.sha256(seed).hexdigest() == '6f9246a6066acabbe71be5419ee390b1767b978c2de80e4434b6f4afbbbf0a05'
at = 0
seen_fragment = False
while at < len(seed):
    size = int.from_bytes(seed[at:at+4], 'big')
    tag = seed[at+4:at+8]
    if size == 1:
        size = int.from_bytes(seed[at+8:at+16], 'big')
    assert size >= 8 and at+size <= len(seed)
    seen_fragment |= tag == b'moof'
    at += size
    if tag == b'mdat' and seen_fragment:
        break
assert seen_fragment
(root / 'curves-matrix-unspecified.mp4').write_bytes(seed[:at])
