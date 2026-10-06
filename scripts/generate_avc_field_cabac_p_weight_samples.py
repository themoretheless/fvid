#!/usr/bin/env python3
"""Owned CABAC I/P fields, weighted references, residual and skip and JM reference."""
import argparse,tempfile,subprocess,hashlib,json
from pathlib import Path
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from avc_fixture_mp4 import mux,annexb

def prediction(bottom,index,address,mode,deblock,init,identity):
    rotate=False;reorder=True
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(1);b.ue(3);b.u(int(reorder))
    if reorder:
        for op,value in [(0,3),(0,0),(1,2),(0,0)]:b.ue(op);b.ue(value)
        b.ue(3)
    b.ue(2);b.ue(1)
    table=[((3,5),[(1,-7),(3,8)]),((-2,90),[(0,24),(-1,48)]),(None,None),((12,-24),[(5,-50),(-2,80)])]
    for luma,chroma in table:
        b.u(int(not identity and luma is not None))
        if not identity and luma is not None:b.se(luma[0]);b.se(luma[1])
        b.u(int(not identity and chroma is not None))
        if not identity and chroma is not None:
            for weight,offset in chroma:b.se(weight);b.se(offset)
    b.u(0);b.ue(init);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(11,int(mode=='skip'))
    if mode=='skip':
        b.bits.extend(c.finish());return b.nal(0x41,trailing=False)
    c.decision(14,0)
    if mode=='16x8':c.decision(15,1);c.decision(17,1);count=2
    elif mode=='8x16':c.decision(15,1);c.decision(17,0);count=2
    else:
        c.decision(15,0);c.decision(16,1)
        code=0
        for _ in range(4):
            for i,symbol in enumerate(['1','00','011','010'][code]):c.decision(21+i,int(symbol))
        count=4
    origins=[(0,0),(0,2)] if mode=='16x8' else [(0,0),(2,0)] if mode=='8x16' else [(0,0),(2,0),(0,2),(2,2)]
    width,height=(4,2) if mode=='16x8' else (2,4) if mode=='8x16' else (2,2)
    references=[None]*16
    for group,(x,y) in enumerate(origins):
        left=references[y*4+x-1] if x else None
        top=references[(y-1)*4+x] if y else None
        inc=int(left is not None and left>0)+2*int(top is not None and top>0)
        reference=(address*2+group+int(rotate))%4
        for symbol in range(reference+1):c.decision(54+inc if symbol==0 else 58 if symbol==1 else 59,int(symbol<reference))
        for dy in range(height):
            for dx in range(width):references[(y+dy)*4+x+dx]=reference
    for part in range(count):
        c.mvd(0,1 if part%2==0 else -1,0);c.mvd(1,-1 if part%2==0 else 1,0)
    if mode!='residual':
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    else:
        for _ in range(4):c.decision(73,1)
        c.decision(77,1);c.decision(81,1);c.decision(60,0)
        done=set()
        for block in range(16):
            bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
            inc=int((bx-1,by) in done)+2*int((bx,by-1) in done)
            c.decision(93+inc,1)
            c.decision(306,0);c.decision(307,1);c.decision(368,1);c.decision(248,0);c.bypass(block%2)
            done.add((bx,by))
        for component in range(2):
            c.decision(97,1)
            c.decision(321,1);c.decision(382,1);c.decision(258,0);c.bypass(component)
        for component in range(2):
            done=set()
            for block in range(4):
                bx=block%2;by=block//2
                inc=int((bx-1,by) in done)+2*int((bx,by-1) in done)
                c.decision(101+inc,1)
                c.decision(324,1);c.decision(385,1);c.decision(267,0);c.bypass((block+component)%2)
                done.add((bx,by))
    b.bits.extend(c.finish());return b.nal(0x41,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-cabac-p-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,mode,deblock,aso,init,identity in [(dep,r,m,f,a,i,t) for dep in [8,10] for r in [False,True] for m in ['skip','16x8','8x8','residual'] for f in [0,1,2] for a in [False,True] for i in range(3) for t in [False,True]]:
            frames=[];order=[True,False] if reverse else [False,True]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    index=2*number+i
                    nals=[field(bottom,index,address,'positive' if (address==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for address in ([1,0] if aso else [0,1])]
                    frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            for n,bottom in enumerate(order):
                nals=[prediction(bottom,4+n,address,mode,deblock,init,identity) for address in ([1,0] if aso else [0,1])]
                frames.append((4+n,False,b''.join(len(nal).to_bytes(4,'big')+nal for nal in nals)))
            config=configuration(depth,weighted=True);name=f'avc-field-cabac-p-weight-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-'+('identity' if identity else 'weighted')+f'-init{init}-filter{deblock}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-cabac-p-weight-generated.json').write_text(json.dumps(dict(generator='owned CABAC weighted field-reference writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
