#!/usr/bin/env python3
"""Original SI macroblock and ordinary I-type control with normative scalar YUV."""
import json
from generate_avc_switching_luma_fixtures import DEST, reference
from generate_avc_secondary_sp_fixtures import chroma
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux


def wide_configuration(constrained):
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8)
    for v in [0,0,0,0,1]:b.ue(v)
    b.u(0);b.ue(1);b.ue(0);b.u(1);b.u(1);b.u(0);b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0)
    for _ in range(3):b.ue(0)
    b.u(0);b.u(0,2)
    for _ in range(3):b.se(0)
    b.u(1);b.u(int(constrained));b.u(0);pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps


def mixed_neighbours(constrained):
    config=wide_configuration(constrained);frames=[];packets=[]
    for index in range(3):
        b=Writer();b.ue(0);b.ue(4);b.ue(0);b.u(index,4)
        if index==0:b.ue(0)
        b.u(index*2,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.se(-26);b.ue(1)
        for mb_type in [0,1]:
            b.ue(mb_type)
            for _ in range(16):b.u(1)
            b.ue(0);b.ue(3)
        nal=b.nal(0x65 if index==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal;packets.append(packet.hex());frames.append((index,index==0,packet))
    left=chroma([128]*64,[0]*4,[[0]*16 for _ in range(4)],0)
    right=[128]*64 if constrained else left
    plane=[v for y in range(8) for v in left[y*8:y*8+8]+right[y*8:y*8+8]]
    gold=bytes([128]*512+plane+plane)*3
    name=f'avc-si-mixed-neighbours-constrained{int(constrained)}'
    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,16,30));(DEST/(name+'-reference.yuv')).write_bytes(gold)
    return dict(configuration=config.hex(),packets=packets,file=name+'-synthetic.mp4',reference=name+'-reference.yuv',kind=name)


def main():
    base=json.loads((DEST/'avc-switching-luma.json').read_text())
    config=bytes.fromhex(base['video']['configuration']);cases=[]
    for kind in ['switching-zero','switching-signed','ordinary-intra-zero']:
        switching=kind!='ordinary-intra-zero';coded=kind=='switching-signed'
        frames=[];packets=[];gold=bytearray()
        for index,qs in enumerate([0,26,51]):
            b=Writer();b.ue(0);b.ue(4);b.ue(0);b.u(index,4)
            if index==0:b.ue(0)
            b.u(index*2,4)
            if index==0:b.u(0);b.u(0)
            else:b.u(0)
            b.se(0);b.se(qs-26);b.ue(1);b.ue(0 if switching else 1)
            for _ in range(16):b.u(1) # predicted DC mode
            b.ue(0);b.ue(0 if coded else 3)
            if coded:
                b.se(0)
                for block in range(16):b.u(1,2);b.u(block%2);b.u(1)
                for component in range(2):b.u(1);b.u(component);b.u(1,2)
                for component in range(2):
                    for block in range(4):b.u(1,2);b.u((block+component)%2);b.u(1)
            nal=b.nal(0x65 if index==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal;packets.append(packet.hex());frames.append((index,index==0,packet))
            luma=[0]*256;ready=set()
            for block in range(16):
                bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2;ox=bx*4;oy=by*4
                top=[luma[(oy-1)*16+ox+x] for x in range(4)] if by>0 and (bx,by-1) in ready else []
                left=[luma[(oy+y)*16+ox-1] for y in range(4)] if bx>0 and (bx-1,by) in ready else []
                neighbours=top+left
                dc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 128
                levels=[(1 if block%2==0 else -1) if coded else 0]+[0]*15
                samples=reference([dc]*16,levels,26,qs,True) if switching else [dc]*16
                for i,v in enumerate(samples):luma[(oy+i//4)*16+ox+i%4]=v
                ready.add((bx,by))
            out=luma
            for component in range(2):
                dc=[0,0,(1 if component==0 else -1) if coded else 0,0]
                ac=[[0,(1 if (block+component)%2==0 else -1) if coded else 0]+[0]*14 for block in range(4)]
                out+=chroma([128]*64,dc,ac,39 if qs==51 else qs) if switching else [128]*64
            gold.extend(out)
        name='avc-si-'+kind
        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,16,16,30));(DEST/(name+'-reference.yuv')).write_bytes(gold)
        cases.append(dict(configuration=config.hex(),packets=packets,file=name+'-synthetic.mp4',reference=name+'-reference.yuv',kind=kind))
    cases.extend(mixed_neighbours(c) for c in [False,True])
    (DEST/'avc-si.json').write_text(json.dumps(dict(cases=cases,provenance='Own Extended-profile SI syntax including sixteen intra4 mode flags required by table7-12 and macroblock prediction syntax7.3.5.1. Independent spatial DC prediction and normative matrix switching oracle, QS0/26/51. Ordinary I_NxN SI-slice control must not use switching. JM19 skips SI4 mode parsing, so is not the syntax/pixel oracle for SI4. No private media, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
