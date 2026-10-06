#!/usr/bin/env python3
"""Owned CABAC separate B fields mixing direct and every explicit B8x8 subtype."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration, field
from generate_avc_field_cabac_p_multiref_samples import prediction as p_field
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb

SUB=['0','100','101','11000','11001','11010','11011','111000','111001','111010','111011','11110','11111']
def b_field(bottom,index,spatial,code,position,vector,deblock,init,address,joined=False,references=False):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0);b.ue(init);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);grids=[{},{}];refgrids=[{},{}]
    for mb in range(2 if joined else 1):
        c.decision(24+mb,0)
        for ctx,v in zip([27+mb,30,31,32,32,32],[1,1,1,1,1,1]):c.decision(ctx,v)
        codes=[0 if g==position else code for g in range(4)];parts=[]
        for g,t in enumerate(codes):
            prior=0
            for i,v in enumerate(SUB[t]):
                ctx=36 if i==0 else 37 if i==1 else (38 if prior&1 else 39) if i==2 else 39
                c.decision(ctx,int(v));prior=prior*2+int(v)
            if t==0:continue
            mode=t if t<=3 else (t-4)//2+1 if t<=9 else t-9
            w,h=(2,2) if t<=3 else (2,1) if t in [4,6,8] else (1,2) if t<=9 else (1,1)
            for y in range(0,2,h):
                for x in range(0,2,w):parts.append((g,(mb*4+g%2*2+x,g//2*2+y),(w,h),mode))
        for li in range(2):
            for g,t in enumerate(codes):
                if not any(p[0]==g and p[3] in [li+1,3] for p in parts):continue
                x,y=mb*4+g%2*2,g//2*2;grid=refgrids[li]
                positive=[grid.get(n,0)>0 for n in [(x-1,y),(x,y-1)]]
                reference=(g+li+mb)%4 if references else 0
                for value in range(reference+1):
                    ctx=54+int(positive[0])+2*int(positive[1]) if value==0 else 58 if value==1 else 59
                    c.decision(ctx,int(value<reference))
                for yy in range(y,y+2):
                    for xx in range(x,x+2):grid[xx,yy]=reference
        for li in range(2):
            grid=grids[li]
            for g,(x,y),(w,h),mode in parts:
                if mode not in [li+1,3]:continue
                mv=((3 if li==0 else -2)*vector,(-2 if li==0 else 3)*vector)
                for component,value in enumerate(mv):
                    mag=sum(grid.get(n,(0,0))[component] for n in [(x-1,y),(x,y-1)])
                    c.mvd(component,value,mag)
                for yy in range(y,y+h):
                    for xx in range(x,x+w):grid[xx,yy]=tuple(abs(v) for v in mv)
        for ctx in ([73,74,75,76,77] if mb==0 else [74,74,76,76,77]):c.decision(ctx,0)
        if joined and mb==0:c.range-=2;c.renormalize()
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)

def main():
    a=argparse.ArgumentParser(description=__doc__);a.add_argument('--jm-decoder',type=Path,required=True);a.add_argument('--smoke',action='store_true');a.add_argument('--joined',action='store_true');a.add_argument('--references',action='store_true');args=a.parse_args();assert not args.references or args.joined
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    dimensions=([8,10],[False,True],[False,True],[False,True],range(3),[0,1,2],range(1,13),range(4))
    if args.smoke:dimensions=([8],[False],[False],[True],[0],[1],range(1,13),range(4))
    with tempfile.TemporaryDirectory(prefix='fvid-cabac-sub-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,spatial,inference,init,deblock,code,position in itertools.product(*dimensions):
            frames=[];order=[True,False] if reverse else [False,True]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    index=number*4+i;ns=[field(bottom,index,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for a in [0,1]];frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            for i,bottom in enumerate(order):
                ns=[p_field(bottom,8+i,a,'4x4',deblock,init,True,True) for a in [0,1]];frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            for i,bottom in enumerate(order):
                ns=[b_field(bottom,6+i,spatial,code,position,1,deblock,init,a,args.joined,args.references) for a in ([0] if args.joined else [0,1])];frames.append((6+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            config=configuration(depth,direct8=inference);family='avc-field-b-cabac-sub-references-' if args.references else 'avc-field-b-cabac-sub-joined-' if args.joined else 'avc-field-b-cabac-sub-';name=f'{family}{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+f'-type{code}-pos{position}-init{init}-filter{deblock}-infer{int(inference)}'
            coded=d/'sample.264';oracle=d/'sample.yuv';coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1)
            data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    if not args.smoke:(root/(family+'generated.json')).write_text(json.dumps(dict(generator='owned CABAC B8x8 fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
