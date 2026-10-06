#!/usr/bin/env python3
"""Owned PAFF CABAC frame I16 slices; explicit JM reference generation."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb

def frame_slice(address, mode, deblock):
    b=Writer(); b.ue(address); b.ue(2); b.ue(0); b.u(0,4); b.u(0); b.ue(0); b.u(0,4); b.u(0); b.u(0); b.se(24); b.ue(deblock)
    if deblock!=1: b.se(6); b.se(6)
    while len(b.bits)%8: b.u(1)
    c=CabacWriter(-1,50); c.decision(3,1); c.range-=2; c.renormalize()
    c.decision(6,0); c.decision(7,0); c.decision(9,1); c.decision(10,0)
    c.decision(64,0); c.decision(60,0); c.decision(88,int(mode!='zero'))
    if mode!='zero':
        c.decision(105,1); c.decision(166,1); c.decision(228,0); c.bypass(int(mode=='negative'))
    b.bits.extend(c.finish()); return b.nal(0x65,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-cabac-') as tmp:
        d=Path(tmp); cfg=d/'decoder.cfg'; cfg.write_text('')
        for depth,mode,filter,aso in itertools.product([8,10],['zero','positive','negative'],range(3),[False,True]):
            config=configuration(depth); nals=[frame_slice(a,mode,filter) for a in ([3,1,2,0] if aso else range(4))]
            frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in nals))]
            name=f'avc-paff-cabac-{depth}bit-{mode}-filter{filter}'+('-aso' if aso else '')
            coded=d/(name+'.264'); oracle=d/(name+'.yuv'); coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes(); assert len(pixels)==1536*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50); (root/(name+'.mp4')).write_bytes(data); (root/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-cabac-generated.json').write_text(json.dumps(dict(generator='owned PAFF CABAC I16 frame writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
