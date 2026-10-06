#!/usr/bin/env python3
"""Owned CABAC I fields with signed field-scanned DC; explicit JM oracle."""
import argparse,tempfile,subprocess,hashlib,json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from avc_fixture_mp4 import mux,annexb

def configuration(depth, constrained=False, weighted=False, bipred=0, direct8=True,max_refs=3):
    profile=100 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(max_refs);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(int(direct8));b.u(0);b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(1);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(int(constrained));b.u(0);pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def field(bottom,index,address,mode,deblock,biased=False,frame_num=0,long_term=False):
    b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(frame_num,4);b.u(1);b.u(int(bottom))
    if index==0:b.ue(0)
    b.u(index,4)
    if index==0:b.u(0);b.u(int(long_term))
    elif long_term:b.u(1);b.ue(6);b.ue(0);b.ue(0)
    else:b.u(0)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(-1,50);c.decision(3,1)
    c.range-=2;c.renormalize() # mb_type terminate=0 (not PCM)
    c.decision(6,0);c.decision(7,0);c.decision(9,1);c.decision(10,0) # I16 DC, CBP0
    c.decision(64,0);c.decision(60,0) # chroma DC prediction, zero QP delta
    c.decision(88,int(mode!='zero'))
    if mode!='zero':
        c.decision(277,int(biased))
        if biased:c.decision(338,0)
        c.decision(278,1);c.decision(339,1);c.decision(228,0);c.bypass(int(mode=='negative'))
        if biased:c.decision(229,0);c.bypass(int(mode=='negative'))
    b.bits.extend(c.finish())
    return b.nal(0x65 if index==0 else 0x41,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-cabac-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,mode,deblock,aso in [(dep,r,m,f,a) for dep in [8,10] for r in [False,True] for m in ['zero','positive','negative'] for f in [0,1,2] for a in [False,True]]:
            frames=[]
            for index,bottom in enumerate([True,False] if reverse else [False,True]):
                nals=[field(bottom,index,address,mode,deblock) for address in ([1,0] if aso else [0,1])]
                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            config=configuration(depth);name=f'avc-field-cabac-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-filter{deblock}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==1536*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-cabac-generated.json').write_text(json.dumps(dict(generator='owned CABAC I16 field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
