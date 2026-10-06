#!/usr/bin/env python3
"""Owned complementary I/P fields with explicit JM reference generation."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import field, Writer
from avc_fixture_mp4 import mux, annexb

def configuration(depth,scaled=False):
    profile=100 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    b.u(1);b.u(int(scaled))
    if scaled:
        for index in range(8):
            b.u(1);value=24 if index in [3,4,5,7] else 8
            b.se(value-8)
            for _ in range((16 if index<6 else 64)-1):b.se(0)
    b.se(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def prediction(bottom, eight, qp):
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(2+int(bottom),4)
    b.u(0);b.u(0);b.u(0);b.se(qp-26);b.ue(1)
    b.ue(0);b.ue(0);b.se(1);b.se(-1);b.ue(12);b.u(int(eight));b.se(3 if bottom else -3)
    for i in range(16):b.u(1,2);b.u(i%2);b.u(3,3)
    for c in range(2):b.u(1);b.u(c);b.u(1)
    for c in range(2):
        for i in range(4):b.u(1,2);b.u((i+c)%2);b.u(1)
    b.ue(1)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-skip-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse,eight,qp,scaled in [(r,e,q,s) for r in [False,True] for e in [False,True] for q in [18,26,40] for s in [False,True]]:
                order=[True,False] if reverse else [False,True]
                nals=[field(bottom,depth,i==0) for i,bottom in enumerate(order)]+[prediction(bottom,eight,qp) for bottom in order]
                frames=[(i,i==0,len(n).to_bytes(4,'big')+n) for i,n in enumerate(nals)]
                config=configuration(depth,scaled);name=f'avc-field-transform-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-t{8 if eight else 4}-qp{qp}'+('-scale24' if scaled else '')
                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                pixels=oracle.read_bytes();assert len(pixels)==3072*(2 if depth>8 else 1),(name,len(pixels))
                data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-transform-generated.json').write_text(json.dumps(dict(generator='owned I/P field transform/QP writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
