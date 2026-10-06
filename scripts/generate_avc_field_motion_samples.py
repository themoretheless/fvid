#!/usr/bin/env python3
"""Owned complementary I/P fields with explicit JM reference generation."""
import argparse, tempfile, subprocess, json, hashlib
from pathlib import Path
from generate_avc_field_pcm_samples import configuration, field, Writer
from avc_fixture_mp4 import mux, annexb

def prediction(bottom, vector):
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(2+int(bottom),4)
    b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
    for i in range(2):
        b.ue(0);b.ue(0);b.se(vector[0] if i==0 else 0);b.se(vector[1] if i==0 else 0);b.ue(0)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-skip-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse,vector in [(r,v) for r in [False,True] for v in [(1,1),(-1,-1),(5,-3),(-7,6)]]:
                order=[True,False] if reverse else [False,True]
                nals=[field(bottom,depth,i==0) for i,bottom in enumerate(order)]+[prediction(bottom,vector) for bottom in order]
                frames=[(i,i==0,len(n).to_bytes(4,'big')+n) for i,n in enumerate(nals)]
                config=configuration(depth);name=f'avc-field-motion-{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-x{vector[0]}-y{vector[1]}'
                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                pixels=oracle.read_bytes();assert len(pixels)==3072*(2 if depth>8 else 1),(name,len(pixels))
                data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-field-motion-generated.json').write_text(json.dumps(dict(generator='owned I/P fractional field motion writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
