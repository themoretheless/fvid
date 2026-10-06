#!/usr/bin/env python3
"""Owned complementary I/P fields with explicit JM reference generation."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, pcm_samples
from avc_fixture_mp4 import mux, annexb

def configuration(depth):
    profile=88 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    if depth>8:b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(1);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def field(bottom,depth,idr,convert_first=False):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
    if idr:b.ue(0)
    b.u(0 if idr else 1,4)
    if idr:b.u(0);b.u(0)
    elif convert_first:
        b.u(1);b.ue(4);b.ue(3);b.ue(3);b.ue(0);b.ue(2);b.ue(0) # MMCO 4 establishes limit, then MMCO 3: previous opposite field -> long-term index 2
    else:b.u(0)
    b.se(0);b.ue(1)
    for address in range(4):
        b.ue(25);b.align();b.bits.extend(pcm_samples(depth,int(bottom)+2*(address//2),address%2))
    return b.nal(0x65 if idr else 0x41)


def prediction(bottom, mode, deblock, addresses):
    b=Writer();b.ue(addresses[0]);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(2+int(bottom),4)
    b.u(0);b.u(0);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for address in addresses:
        b.ue(0);b.ue(0)
        if mode=='motion':b.se(0);b.se((address//2)*2);b.ue(0)
        else:b.se(1);b.se(-1);b.ue({'luma-ac':11,'all-ac':12}[mode]);b.se(0)
        if mode in ['luma-ac','all-ac']:
            for i in range(16):b.u(1,2);b.u(i%2);b.u(3,3)
        if mode in ['chroma-dc','all-ac']:
            for c in range(2):b.u(1);b.u(c);b.u(1)
        if mode=='all-ac':
            for c in range(2):
                for i in range(4):b.u(1,2);b.u((i+c)%2);b.u(1)
    return b.nal(0x41 if addresses[0]==0 else 0x61)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-skip-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse,mode,deblock,layout,aso in [(r,m,f,l,a) for r in [False,True] for m in ['motion','all-ac'] for f in [0,1,2] for l in ['one','rows','blocks'] for a in ([False] if l=='one' else [False,True])]:
                order=[True,False] if reverse else [False,True]
                frames=[]
                for i,bottom in enumerate(order):
                    n=field(bottom,depth,i==0);frames.append((i,i==0,len(n).to_bytes(4,'big')+n))
                for i,bottom in enumerate(order):
                    groups={'one':[[0,1,2,3]],'rows':[[0,1],[2,3]],'blocks':[[0],[1],[2],[3]]}[layout]
                    nals=[prediction(bottom,mode,deblock,addresses) for addresses in (groups[::-1] if aso else groups)]
                    frames.append((i+2,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                config=configuration(depth);name=f'avc-field-rows-{depth}bit-'+('bottom-first' if reverse else 'top-first')+'-'+mode+f'-{layout}-filter{deblock}'+('-aso' if aso else '')
                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth>8 else 1),(name,len(pixels))
                data=mux(config,frames,32,64,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-rows-generated.json').write_text(json.dumps(dict(generator='owned I/P multi-row field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
