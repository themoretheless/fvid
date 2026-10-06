#!/usr/bin/env python3
"""Owned CABAC I/P fields, explicit skip/fractional motion and JM reference."""
import argparse,tempfile,subprocess,hashlib,json
from pathlib import Path
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from avc_fixture_mp4 import mux,annexb

def prediction(bottom,index,address,mode,deblock,init):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(1);b.ue(0);b.u(0);b.u(0);b.ue(init);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(11,0);c.decision(14,0)
    if mode=='16x8':c.decision(15,1);c.decision(17,1);count=2
    elif mode=='8x16':c.decision(15,1);c.decision(17,0);count=2
    else:
        c.decision(15,0);c.decision(16,1)
        code={'8x8':0,'8x4':1,'4x8':2,'4x4':3}[mode]
        for _ in range(4):
            for i,symbol in enumerate(['1','00','011','010'][code]):c.decision(21+i,int(symbol))
        count={'8x8':4,'8x4':8,'4x8':8,'4x4':16}[mode]
    for part in range(count):
        c.mvd(0,1 if part%2==0 else -1,0);c.mvd(1,-1 if part%2==0 else 1,0)
    for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x41,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-cabac-p-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,mode,deblock,aso,init in [(dep,r,m,f,a,i) for dep in [8,10] for r in [False,True] for m in ['16x8','8x16','8x8','8x4','4x8','4x4'] for f in [0,1,2] for a in [False,True] for i in range(3)]:
            frames=[];order=[True,False] if reverse else [False,True]
            for index,bottom in enumerate(order):
                nals=[field(bottom,index,address,'positive' if address==0 else 'negative',deblock) for address in ([1,0] if aso else [0,1])]
                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            for n,bottom in enumerate(order):
                nals=[prediction(bottom,2+n,address,mode,deblock,init) for address in ([1,0] if aso else [0,1])]
                frames.append((2+n,False,b''.join(len(nal).to_bytes(4,'big')+nal for nal in nals)))
            config=configuration(depth);name=f'avc-field-cabac-p-partition-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-init{init}-filter{deblock}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==3072*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-cabac-p-partition-generated.json').write_text(json.dumps(dict(generator='owned CABAC fractional P field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
