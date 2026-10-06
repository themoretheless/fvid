#!/usr/bin/env python3
"""Owned mixed intra residual/inter P fields; explicitly generated independent JM oracle."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import configuration, Writer
from generate_avc_field_reference_samples import intra
from generate_avc_mbaff_direct_samples import pcm_samples
from avc_fixture_mp4 import mux, annexb

def prediction(bottom, depth, pcm_first, skipped, deblock, poc, residual):
    b=Writer(); b.ue(0); b.ue(0); b.ue(0); b.u(1,4); b.u(1); b.u(int(bottom)); b.u(poc,4)
    b.u(0); b.u(0); b.u(0); b.se(24); b.ue(deblock)
    if deblock!=1: b.se(6); b.se(6)
    for address in range(2):
        if (address==0)==pcm_first:
            
            if not (address==1 and skipped): b.ue(0)
            if residual.startswith('i4'):
                b.ue(5)
                for _ in range(16): b.u(1)
                b.ue(0); b.ue(2); b.se(0) # intra CBP15, QP unchanged
                for i in range(16):
                    if residual=='i4-zero': b.u(1)
                    else: b.u(1,2); b.u(i%2); b.u(3,3) # one signed AC at scan index one
            else:
                b.ue(8); b.ue(0); b.se(0) # I16 DC, CBP0
                if residual=='i16-zero': b.u(1)
                else: b.u(1,2); b.u(int(residual=='i16-negative')); b.u(1)
        elif skipped: b.ue(1)
        else: b.ue(0); b.ue(0); b.se(1); b.se(-1); b.ue(0)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-intra-residual-') as tmp:
        d=Path(tmp); cfg=d/'decoder.cfg'; cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for first in [False,True]:
                    for skipped in [False,True]:
                        for deblock,residual in [(f,r) for f in [0,1,2] for r in ["i4-zero","i4-ac","i16-zero","i16-positive","i16-negative"]]:
                            frames=[]; order=[True,False] if reverse else [False,True]
                            for i,bottom in enumerate(order):
                                n=intra(bottom,depth,0,i==0,i); frames.append((i,i==0,len(n).to_bytes(4,'big')+n))
                            for i,bottom in enumerate(order):
                                n=prediction(bottom,depth,first,skipped,deblock,2+i,residual); frames.append((2+i,False,len(n).to_bytes(4,'big')+n))
                            name=f'avc-field-intra-residual-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-intra-first' if first else '-intra-last')+('-skip' if skipped else '-coded')+f'-{residual}-filter{deblock}'
                            config=configuration(depth); coded=d/(name+'.264'); oracle=d/(name+'.yuv'); coded.write_bytes(annexb(config,frames))
                            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                            pixels=oracle.read_bytes(); assert len(pixels)==3072*(2 if depth>8 else 1)
                            data=mux(config,frames,32,32,50); (out/(name+'.mp4')).write_bytes(data); (out/(name+'.yuv')).write_bytes(pixels)
                            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-intra-residual-generated.json').write_text(json.dumps(dict(generator='owned mixed intra residual/inter field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
