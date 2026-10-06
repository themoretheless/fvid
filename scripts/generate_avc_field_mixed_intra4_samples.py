#!/usr/bin/env python3
"""Owned mixed Intra4/inter P fields; explicitly generated independent JM oracle."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import configuration, Writer
from generate_avc_field_reference_samples import intra
from generate_avc_mbaff_direct_samples import pcm_samples
from avc_fixture_mp4 import mux, annexb

def prediction(bottom, depth, pcm_first, skipped, deblock, poc):
    b=Writer(); b.ue(0); b.ue(0); b.ue(0); b.u(1,4); b.u(1); b.u(int(bottom)); b.u(poc,4)
    b.u(0); b.u(0); b.u(0); b.se(24); b.ue(deblock)
    if deblock!=1: b.se(6); b.se(6)
    for address in range(2):
        if (address==0)==pcm_first:
            
            if not (address==1 and skipped): b.ue(0)
            b.ue(5)
            for _ in range(16): b.u(1)
            b.ue(0); b.ue(3)
        elif skipped: b.ue(1)
        else: b.ue(0); b.ue(0); b.se(1); b.se(-1); b.ue(0)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-mixed-intra4-') as tmp:
        d=Path(tmp); cfg=d/'decoder.cfg'; cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for first in [False,True]:
                    for skipped in [False,True]:
                        for deblock in [0,1,2]:
                            frames=[]; order=[True,False] if reverse else [False,True]
                            for i,bottom in enumerate(order):
                                n=intra(bottom,depth,0,i==0,i); frames.append((i,i==0,len(n).to_bytes(4,'big')+n))
                            for i,bottom in enumerate(order):
                                n=prediction(bottom,depth,first,skipped,deblock,2+i); frames.append((2+i,False,len(n).to_bytes(4,'big')+n))
                            name=f'avc-field-mixed-intra4-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-intra-first' if first else '-intra-last')+('-skip' if skipped else '-coded')+f'-filter{deblock}'
                            config=configuration(depth); coded=d/(name+'.264'); oracle=d/(name+'.yuv'); coded.write_bytes(annexb(config,frames))
                            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                            pixels=oracle.read_bytes(); assert len(pixels)==3072*(2 if depth>8 else 1)
                            data=mux(config,frames,32,32,50); (out/(name+'.mp4')).write_bytes(data); (out/(name+'.yuv')).write_bytes(pixels)
                            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-mixed-intra4-generated.json').write_text(json.dumps(dict(generator='owned mixed Intra4/inter field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
