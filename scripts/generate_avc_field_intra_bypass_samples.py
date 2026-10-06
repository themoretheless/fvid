#!/usr/bin/env python3
"""Owned intra bypass I fields, explicit separate JM reference generation."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import Writer
from generate_avc_field_bypass_samples import configuration
from avc_fixture_mp4 import mux, annexb

def field(bottom,index,address,mode,deblock,depth):
    b=Writer(); b.ue(address); b.ue(2); b.ue(0); b.u(0,4); b.u(1); b.u(int(bottom))
    if index==0: b.ue(0)
    b.u(index,4)
    if index==0: b.u(0); b.u(0)
    else: b.u(0)
    b.se(-6*(depth-8)-26); b.ue(deblock)
    if deblock!=1: b.se(6); b.se(6)
    if mode.startswith(('i4','i8')):
        b.ue(0); b.u(int(mode.startswith('i8')))
        for _ in range(4 if mode.startswith('i8') else 16): b.u(1)
        b.ue(0); b.ue(0); b.se(0)
        for i in range(16):
            if mode.endswith('zero'): b.u(1)
            else: b.u(1,2); b.u((i+address+index)%2); b.u(3,3)
    else:
        b.ue(11); b.ue(0); b.se(0)
        if mode.endswith('zero'): b.u(1)
        else: b.u(1,2); b.u(int(mode=='i16-negative')); b.u(1)
    for c in range(2):
        if mode.endswith('zero'): b.u(1,2)
        else: b.u(1); b.u(c); b.u(1)
    for c in range(2):
        for i in range(4):
            if mode.endswith('zero'): b.u(1)
            else: b.u(1,2); b.u((i+c)%2); b.u(1)
    return b.nal(0x65 if index==0 else 0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-i-fields-') as tmp:
        d=Path(tmp); cfg=d/'decoder.cfg'; cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for mode in ['i4-zero','i4-ac','i8-zero','i8-ac','i16-zero','i16-positive','i16-negative']:
                    for deblock in [1]:
                        for aso,scaled,bypass in [(a,s,b) for a in [False,True] for s in [False,True] for b in [False,True]]:
                            frames=[]
                            for index,bottom in enumerate([True,False] if reverse else [False,True]):
                                nals=[field(bottom,index,address,mode,deblock,depth) for address in ([1,0] if aso else [0,1])]
                                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                            name=f'avc-field-intra-bypass-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-filter{deblock}'+('-scale8' if scaled else '-scale16')+('-enabled' if bypass else '-control')+('-aso' if aso else '')
                            config=configuration(depth,scaled,bypass); coded=d/(name+'.264'); oracle=d/(name+'.yuv'); coded.write_bytes(annexb(config,frames))
                            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                            pixels=oracle.read_bytes(); assert len(pixels)==1536*(2 if depth>8 else 1),(name,len(pixels))
                            data=mux(config,frames,32,32,50); (out/(name+'.mp4')).write_bytes(data); (out/(name+'.yuv')).write_bytes(pixels)
                            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-intra-bypass-generated.json').write_text(json.dumps(dict(generator='owned CAVLC non-PCM field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
