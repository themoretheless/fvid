#!/usr/bin/env python3
"""Owned CABAC I/P fields, explicit signed field residual and JM reference."""
import argparse,tempfile,subprocess,hashlib,json
from pathlib import Path
from generate_avc_field_cabac_samples import field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from avc_fixture_mp4 import mux,annexb

def configuration(depth,scaled=False,bypass=False,high444=False):
    profile=244 if bypass or high444 else 100 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(int(bypass));b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(1);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    b.u(1);b.u(int(scaled))
    if scaled:
        for index in range(8):
            b.u(1);value=24 if index in [3,4,5,7] else 16
            b.se(value-8)
            for _ in range((16 if index<6 else 64)-1):b.se(0)
    b.se(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def prediction(bottom,index,address,mode,deblock,init):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(1);b.ue(0);b.u(0);b.u(0);b.ue(init);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(11,0)
    c.decision(14,0);c.decision(15,0);c.decision(16,0)
    c.mvd(0,1,0);c.mvd(1,-1,0)
    if mode=='zero':
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    else:
        for _ in range(4):c.decision(73,1)
        c.decision(77,0);c.decision(399,1);c.decision(60,0)
        for block in range(4):
            c.decision(436,0);c.decision(437,1);c.decision(452,1);c.decision(427,0);c.bypass(block%2)
    b.bits.extend(c.finish());return b.nal(0x41,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-cabac-p-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,mode,deblock,aso,init,scaled in [(dep,r,m,f,a,i,s) for dep in [8,10] for r in [False,True] for m in ['zero','ac'] for f in [0,1,2] for a in [False,True] for i in range(3) for s in [False,True]]:
            frames=[];order=[True,False] if reverse else [False,True]
            for index,bottom in enumerate(order):
                nals=[field(bottom,index,address,'positive' if address==0 else 'negative',deblock) for address in ([1,0] if aso else [0,1])]
                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            for n,bottom in enumerate(order):
                nals=[prediction(bottom,2+n,address,mode,deblock,init) for address in ([1,0] if aso else [0,1])]
                frames.append((2+n,False,b''.join(len(nal).to_bytes(4,'big')+nal for nal in nals)))
            config=configuration(depth,scaled);name=f'avc-field-cabac-p-transform8-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-init{init}-filter{deblock}'+('-scale24' if scaled else '-scale16')+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==3072*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-cabac-p-transform8-generated.json').write_text(json.dumps(dict(generator='owned CABAC fractional P field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
