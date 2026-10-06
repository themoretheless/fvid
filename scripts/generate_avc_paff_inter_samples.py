#!/usr/bin/env python3
"""Owned PAFF CABAC P frame skip/motion slices after signed I16 references."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_avc_paff_cabac_samples import frame_slice
from generate_avc_field_cabac_samples import configuration
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb

def prediction(address, skip, init, filter, poc=2, joined=False):
    b=Writer(); b.ue(address); b.ue(0); b.ue(0); b.u(1,4); b.u(0); b.u(poc,4)
    b.u(0); b.u(0); b.u(0); b.ue(init); b.se(24); b.ue(filter)
    if filter!=1: b.se(6); b.se(6)
    while len(b.bits)%8: b.u(1)
    c=CabacWriter(init,50)
    for at in range(4) if joined else [address]:
        left=joined and at%2!=0; top=joined and at>=2
        c.decision(11+int(not skip)*(int(left)+int(top)),int(skip))
        if not skip:
            c.decision(14,0); c.decision(15,0); c.decision(16,0)
            for component in range(2): c.mvd(component,0 if joined and at else 4,4 if joined and at in [1,2] else 0)
            for block in range(4): c.decision(73+int(left or block%2!=0)+2*int(top or block>=2),0)
            c.decision(77,0)
        if joined and at!=3: c.range-=2; c.renormalize()
    b.bits.extend(c.finish()); return b.nal(0x41,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); p.add_argument("--joined",action="store_true"); args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-inter-') as tmp:
        d=Path(tmp); cfg=d/'decoder.cfg'; cfg.write_text('')
        for depth,init,skip,filter,aso in itertools.product([8,10],range(3),[False,True],range(3),([False] if args.joined else [False,True])):
            config=configuration(depth); frames=[]
            for index in range(2):
                ns=[frame_slice(a,'positive' if a%2==0 else 'negative',filter) if index==0 else prediction(a,skip,init,filter) for a in ([3,1,2,0] if aso else range(4))]
                if args.joined and index==1: ns=[prediction(0,skip,init,filter,joined=True)]
                frames.append((index*2,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            name=f'avc-paff-inter-'+('joined-' if args.joined else '')+f'{depth}bit-init{init}-'+('skip' if skip else 'motion')+f'-filter{filter}'+('-aso' if aso else '')
            coded=d/(name+'.264'); oracle=d/(name+'.yuv'); coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes(); assert len(pixels)==3072*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(config,frames,32,32,50); (root/(name+'.mp4')).write_bytes(data); (root/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/('avc-paff-inter-'+('joined-' if args.joined else '')+'generated.json')).write_text(json.dumps(dict(generator='owned PAFF CABAC P frame writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
