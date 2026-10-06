#!/usr/bin/env python3
"""Owned per-slice field reference-list modifications; explicit JM pixel oracle."""
import argparse,tempfile,subprocess,json,hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import Writer
from generate_avc_mbaff_direct_samples import pcm_samples
from avc_fixture_mp4 import mux,annexb

def configuration(depth):
    profile=88 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    if depth>8:b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(1);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def intra(bottom,depth,number,idr,poc):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(number,4);b.u(1);b.u(int(bottom))
    if idr:b.ue(0)
    b.u(poc,4)
    if idr:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(1)
    for address in range(2):b.ue(25);b.align();b.bits.extend(pcm_samples(depth,int(bottom)+2*number,address))
    return b.nal(0x65 if idr else 0x41)

def prediction(bottom,address,identity,mode,deblock):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(4+int(bottom),4)
    b.u(1);b.ue(3);b.u(1)
    for op,value in [(0,3),(0,0),(1,2),(0,0)]:b.ue(op);b.ue(value)
    b.ue(3);b.ue(2);b.ue(1)
    table=[((3,5),[(1,-7),(3,8)]),((-2,90),[(0,24),(-1,48)]),(None,None),((12,-24),[(5,-50),(-2,80)])]
    for luma,chroma in table:
        b.u(int(not identity and luma is not None))
        if not identity and luma is not None:b.se(luma[0]);b.se(luma[1])
        b.u(int(not identity and chroma is not None))
        if not identity and chroma is not None:
            for w,o in chroma:b.se(w);b.se(o)
    b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if mode=='whole-skip':b.ue(2)
    else:
        code=1 if mode=='16x8' else 3
        b.ue(0);b.ue(code)
        if code==3:
            for _ in range(4):b.ue(0)
        groups=2 if code==1 else 4
        for group in range(groups):b.ue((address*2+group)%4)
        for _ in range(groups):b.se(0);b.se(0)
        b.ue(12 if mode=='residual' else 0)
        if mode=='residual':
            b.se(0)
            for i in range(16):b.u(1,2);b.u(i%2);b.u(3,3)
            for c in range(2):b.u(1);b.u(c);b.u(1)
            for c in range(2):
                for i in range(4):b.u(1,2);b.u((i+c)%2);b.u(1)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-refs-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,identity,mode,deblock,aso in [(d,r,t,m,f,a) for d in [8,10] for r in [False,True] for t in [False,True] for m in ['whole-skip','16x8','8x8','residual'] for f in [0,1,2] for a in ([False] if m=='whole-skip' else [False,True])]:
            order=[True,False] if reverse else [False,True];frames=[]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    n=intra(bottom,depth,number,number==0 and i==0,number*2+i);frames.append((number*2+i,number==0 and i==0,len(n).to_bytes(4,'big')+n))
            for i,bottom in enumerate(order):
                nals=[prediction(bottom,address,identity,mode,deblock) for address in ([0] if mode=='whole-skip' else [1,0] if aso else [0,1])]
                frames.append((4+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            config=configuration(depth);name=f'avc-field-weight-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-'+('identity' if identity else 'weighted')+f'-filter{deblock}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth>8 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-weight-generated.json').write_text(json.dumps(dict(generator='owned weighted field-reference writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
