#!/usr/bin/env python3
"""Own complementary SP/SI fields and scalar normative planar oracle."""
import json
from generate_avc_field_pcm_samples import configuration, Writer
from generate_avc_switching_luma_fixtures import DEST, reference
from generate_avc_switching_chroma_fixtures import chroma as primary_chroma
from generate_avc_secondary_sp_fixtures import chroma as switching_chroma
from avc_fixture_mp4 import mux


def planes(bottom):
    return [[48+bottom*37+(x*5+y*3)%128 for y in range(16) for x in range(32)],
            [64+bottom*19+x*5+y*3 for y in range(8) for x in range(16)],
            [192-bottom*17-x*5-y*3 for y in range(8) for x in range(16)]]


def header(bottom, frame, qs, kind, idr, reverse=False, address=0):
    b=Writer();b.ue(address);b.ue(4 if kind=='si' else (2 if kind=='pcm' else 3));b.ue(0);b.u(frame,4);b.u(1);b.u(int(bottom))
    if idr:b.ue(0)
    b.u(frame*2+int(bottom != reverse),4)
    if kind not in ['si','pcm']:b.u(0);b.u(0)
    if idr:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0)
    if kind not in ['pcm']:
        if kind!='si':b.u(int(kind=='secondary'))
        b.se(qs-26)
    b.ue(1)
    return b


def pcm(bottom,idr,reverse):
    b=header(bottom,0,0,'pcm',idr,reverse);p=planes(bottom)
    for mb in range(2):
        b.ue(25);b.align()
        for plane,w,n in zip(p,[32,16,16],[16,8,8]):
            for y in range(n):
                for x in range(n):b.u(plane[y*w+mb*n+x],8)
    return b.nal(0x65 if idr else 0x41)


def switching(bottom,frame,qs,kind,coded,idr,reverse,address=None,mv=(0,0)):
    b=header(bottom,frame,qs,kind,idr,reverse,address or 0)
    if kind!='si' and not coded and address is None:b.ue(2)
    else:
        for mb in (range(2) if address is None else [address]):
            if kind=='si':
                b.ue(0)
                for _ in range(16):b.u(1)
                b.ue(0);b.ue(0 if coded else 3)
            else:
                b.ue(0);b.ue(0);b.se(mv[0]);b.se(mv[1]);b.ue(12 if coded else 0)
            if coded:
                b.se(0)
                for block in range(16):b.u(1,2);b.u((block+mb)%2);b.u(1)
                for component in range(2):b.u(1);b.u((component+mb)%2);b.u(1,2)
                for component in range(2):
                    for block in range(4):b.u(1,2);b.u((block+component+mb)%2);b.u(1)
    return b.nal(0x65 if idr else 0x41)


def reconstruct(previous, qs, kind, coded):
    out=[[0]*512,[0]*128,[0]*128];ready=set()
    for mb in range(2):
        for block in range(16):
            bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2;x=mb*16+bx*4;y=by*4
            if kind=='si':
                top=[out[0][(y-1)*32+x+i] for i in range(4)] if y and (x//4,y//4-1) in ready else []
                left=[out[0][(y+i)*32+x-1] for i in range(4)] if x and (x//4-1,y//4) in ready else []
                n=top+left;dc=(sum(n)+len(n)//2)//len(n) if n else 128;p=[dc]*16
            else:p=[previous[0][(y+i//4)*32+x+i%4] for i in range(16)]
            r=[(1 if (block+mb)%2==0 else -1) if coded else 0]+[0]*15
            values=reference(p,r,26,qs,kind!='primary')
            for i,v in enumerate(values):out[0][(y+i//4)*32+x+i%4]=v
            ready.add((x//4,y//4))
        for component in range(2):
            plane=out[component+1];x=mb*8
            if kind=='si':
                # No top edge in this one-MB-high compact field. DC chroma uses left.
                p=[]
                for row in range(8):
                    start=row//4*4
                    dc_pred=(sum(plane[y*16+x-1] for y in range(start,start+4))+2)//4 if mb else 128
                    p.extend([dc_pred]*8)
            else:p=[previous[component+1][i//8*16+x+i%8] for i in range(64)]
            dc=[0,0,(1 if (component+mb)%2==0 else -1) if coded else 0,0]
            # Field scan index1 maps to raster coefficient4 (vertical AC).
            ac=[[0]*16 for _ in range(4)]
            if coded:
                for block in range(4):ac[block][4]=1 if (block+component+mb)%2==0 else -1
            qsc=39 if qs==51 else qs
            values=primary_chroma(p,dc,ac,26,qsc) if kind=='primary' else switching_chroma(p,dc,ac,qsc)
            for i,v in enumerate(values):plane[i//8*16+x+i%8]=v
    return out


def weave(fields):
    raw=[]
    for component,(width,height) in enumerate([(32,16),(16,8),(16,8)]):
        for y in range(height):
            for bottom in [False,True]:raw+=fields[bottom][component][y*width:(y+1)*width]
    return bytes(raw)


def main():
    config=configuration(8,max_refs=3);cases=[]
    for kind in ['primary','secondary','si']:
        for coded in [False,True]:
            for reverse in [False,True]:
                order=[True,False] if reverse else [False,True];frames=[];packets=[];gold=bytearray();fields={b:planes(b) for b in order}
                def append(n,idr):
                    packet=len(n).to_bytes(4,'big')+n;frames.append((len(frames),idr,packet));packets.append(packet.hex())
                if kind!='si':
                    for i,bottom in enumerate(order):append(pcm(bottom,i==0,reverse),i==0)
                    gold.extend(weave(fields))
                for index,qs in enumerate([0,26,51]):
                    frame=index+(kind!='si')
                    for i,bottom in enumerate(order):
                        idr=kind=='si' and index==0 and i==0
                        append(switching(bottom,frame,qs,kind,coded,idr,reverse),idr)
                        fields[bottom]=reconstruct(fields[bottom],qs,kind,coded)
                    gold.extend(weave(fields))
                name=f'avc-switching-field-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')
                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
                cases.append(dict(kind=kind,coded=coded,reverse=reverse,configuration=config.hex(),packets=packets,file=name+'-synthetic.mp4',reference=name+'-reference.yuv',frame_count=len(gold)//1536))
    (DEST/'avc-switching-fields.json').write_text(json.dumps(dict(cases=cases,provenance='Own complementary 32x16 compact fields woven into32x32 pictures, both parity orders. Extended-profile primary/secondary SP skip/signed residual and SI zero/signed residual at QS0/26/51. Scalar matrix and spatial DC oracle uses field coefficient scan and normative chroma signed shift/DC copy. No private media, FFmpeg, external encoder or network.'),indent=2)+'\n')
if __name__=='__main__':main()
