#!/usr/bin/env python3
"""Original neutral VP9 pictures: no encoder, private input or network."""
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/fixtures/playback-errors'
def frame(depth, chroma):
    profile = (0 if chroma == '420' else 1) + (2 if depth > 8 else 0)
    bits = '10' + str(profile & 1) + str(profile >> 1)
    if profile == 3: bits += '0'
    bits += '0010'  # coded keyframe, shown, not error resilient
    bits += f'{0x498342:024b}'
    if depth > 8: bits += str(int(depth == 12))
    bits += '0010'  # BT.601, limited range
    if profile & 1: bits += ('000' if chroma == '444' else '100')
    bits += f'{63:016b}{63:016b}' + '0'  # 64 square, render size unchanged
    bits += '0100'  # no refresh context, frame parallel, context zero
    bits += '0' * 10  # loop filter level/sharpness/delta enable
    bits += '0' * 8 + '000' + '0'  # lossless quantizer, no segmentation
    bits += '0' + f'{32:016b}'  # one tile row, 32-byte compressed header
    bits += '0' * (-len(bits) % 8)
    # Zero arithmetic code selects no probability updates, unsplit blocks,
    # DC prediction and immediate EOB for each transform. Padding is zero.
    return bytes(int(bits[i:i+8], 2) for i in range(0,len(bits),8)) + bytes(32+4096)
def main():
    cases=[]
    for depth in [8,10,12]:
        for chroma in ['420','422','444']:
            name=f'vp9-chroma-{chroma}-{depth}-synthetic.vp9'
            (DEST/name).write_bytes(frame(depth,chroma))
            cases.append(dict(file=name,depth=depth,chroma=chroma,size=[64,64],sample=1 << (depth-1)))
    (DEST/'vp9-chroma.json').write_text(json.dumps(dict(cases=cases,provenance='Original uncompressed header and zero Boolean arithmetic code. DC intra prediction with zero residual; all reconstructed samples equal the bit-depth midpoint. No source media, external codec or network.'),indent=2)+'\n')
if __name__ == '__main__': main()
