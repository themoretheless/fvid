#!/usr/bin/env python3
"""Owned FMO I-field PCM/I16 DC; independent JM output generated explicitly."""
import argparse, tempfile, subprocess, hashlib, json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, pcm_samples
from avc_fixture_mp4 import mux, annexb

def configuration(depth,kind,direction):
    profile=88 if depth==8 else 110
    b=Writer(); b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    if depth==10: b.ue(1);b.ue(2);b.ue(2);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(1);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(1);b.ue(kind)
    if kind==0:b.ue(0);b.ue(0)
    elif kind==2:b.ue(0);b.ue(0)
    elif kind in [3,4,5]:b.u(int(direction));b.ue(0)
    elif kind==6:
        b.ue(3)
        for group in [0,1,0,1]:b.u(group)
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def field(addresses,kind,bottom,index,depth,residual,deblock):
    b=Writer();b.ue(addresses[0]);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
    if index==0:b.ue(0)
    b.u(index,4)
    if index==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if kind in [3,4,5]:b.u(2,3)
    done={}
    for n,address in enumerate(addresses):
        if n==0:
            b.ue(25);b.align();b.bits.extend(pcm_samples(depth,address//2+int(bottom)*2,address%2));done[address]=16
        else:
            b.ue(3);b.ue(0);b.se(0)
            neighbours=[done[a] for a in [address-2,address-1 if address%2 else -1] if a in done]
            nc=(sum(neighbours)+1)//2 if len(neighbours)==2 else neighbours[0] if neighbours else 0
            if residual:b.u(1,6 if nc>=8 else 2);b.u(int(bottom));b.u(1)
            else:b.u(3,6) if nc>=8 else b.u(1)
            done[address]=0
    return b.nal(0x65 if index==0 else 0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-fmo-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,kind,direction,reverse,residual,deblock,aso in [(dep,k,di,r,n,f,a) for dep in [8,10] for k in range(7) for di in ([False,True] if k in [3,4,5] else [False]) for r in [False,True] for n in [False,True] for f in [0,1,2] for a in [False,True]]:
            maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]}
            frames=[]
            for index,bottom in enumerate([True,False] if reverse else [False,True]):
                nals=[field([i for i,g in enumerate(maps[kind]) if g==group],kind,bottom,index,depth,residual,deblock) for group in ([1,0] if aso else [0,1])]
                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            config=configuration(depth,kind,direction);name=f'avc-field-fmo-{depth}bit-type{kind}-dir{int(direction)}-'+('bottom-first' if reverse else 'top-first')+f'-dc{int(residual)}-filter{deblock}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==3072*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,64,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-fmo-generated.json').write_text(json.dumps(dict(generator='owned PCM/I16 field FMO writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
