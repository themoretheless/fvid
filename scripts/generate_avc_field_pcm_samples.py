#!/usr/bin/env python3
"""Owned complementary AVC PCM fields; explicit separate JM oracle generation."""
import argparse,json,hashlib,subprocess,tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def configuration(depth, bipred=0):
    profile=88 if depth==8 else 110
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0)
    if depth>8:b.ue(1);b.ue(depth-8);b.ue(depth-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0);b.u(0);b.u(0);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(bipred,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
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
    for address in range(2):
        b.ue(25);b.align();b.bits.extend(pcm_samples(depth,int(bottom),address))
    return b.nal(0x65 if idr else 0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse,convert_first in [(r,c) for r in [False,True] for c in [False,True]]:
                config=configuration(depth);frames=[]
                for index,bottom in enumerate([True,False] if reverse else [False,True]):
                    n=field(bottom,depth,index==0,convert_first);frames.append((index,index==0,len(n).to_bytes(4,'big')+n))
                name=f'avc-field-pcm-{depth}bit'+('-bottom-first' if reverse else '-top-first')+('-mixed-long' if convert_first else '')
                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True)
                pixels=oracle.read_bytes();assert len(pixels)==1536*(2 if depth>8 else 1), (name,len(pixels))
                data=mux(config,frames,32,32,50);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    config=configuration(8);n=field(False,8,True)
    (out/'avc-field-pcm-unpaired.mp4').write_bytes(mux(config,[(0,True,len(n).to_bytes(4,'big')+n)],32,32,50))
    (out/'avc-field-pcm-generated.json').write_text(json.dumps(dict(generator='owned complementary field PCM writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
