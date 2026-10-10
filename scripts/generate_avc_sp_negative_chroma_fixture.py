#!/usr/bin/env python3
"""Own two-frame negative chroma-transform rounding reproducers."""
import json
from generate_avc_switching_luma_fixtures import DEST, reference
from generate_avc_switching_chroma_fixtures import chroma as primary
from generate_avc_secondary_sp_fixtures import chroma as secondary
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux


def main():
    m=json.loads((DEST/'avc-switching-chroma.json').read_text());config=bytes.fromhex(m['video']['configuration']);cases=[]
    source=[128]*256+[32+x*13+y*7 for y in range(8) for x in range(8)]+[224-x*13-y*7 for y in range(8) for x in range(8)]
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(25);b.align()
    for v in source:b.u(v,8)
    nal=b.nal(0x65);first=len(nal).to_bytes(4,'big')+nal
    for switching in [False,True]:
        b=Writer();b.ue(0);b.ue(3);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.u(int(switching));b.se(0);b.ue(1);b.ue(1)
        nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
        luma=reference([128]*16,[0]*16,26,26,switching)[0]
        result=[luma]*256
        for offset in [256,320]:
            result+=secondary(source[offset:offset+64],[0]*4,[[0]*16 for _ in range(4)],26) if switching else primary(source[offset:offset+64],[0]*4,[[0]*16 for _ in range(4)],26,26)
        name='avc-sp-negative-chroma-'+('secondary' if switching else 'primary')
        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,[(0,True,first),(1,False,packet)],16,16,30));(DEST/(name+'-reference.yuv')).write_bytes(bytes(source+result))
        cases.append(dict(configuration=config.hex(),packets=[first.hex(),packet.hex()],file=name+'-synthetic.mp4',reference=name+'-reference.yuv',switching=switching))
    (DEST/'avc-sp-negative-chroma.json').write_text(json.dumps(dict(cases=cases,provenance='Original two-frame I_PCM to SP-skip streams, rising/falling planar chroma produces negative AC and Hadamard DC. Scalar H.264 8-425/429/435/439 keeps sign inside arithmetic division by power of two. No private media, external decoder, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
