#!/usr/bin/env python3
"""Owned CAVLC B-direct fields with signed luma/chroma residual and zero controls."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration
from generate_avc_field_reference_samples import intra
from generate_avc_field_b_direct_samples import p_field
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux, annexb


def b_field(bottom,index,spatial,mode,position,negative,deblock):
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for address in range(2):
        residual=mode if address==position else 'zero'
        b.ue(0);b.ue(0);b.ue({'zero':0,'luma-ac':11,'chroma-dc':1,'all-ac':12}[residual])
        if residual!='zero':b.se(0)
        if residual in ['luma-ac','all-ac']:
            for i in range(16):b.u(1,2);b.u((i%2)^negative);b.u(3,3)
        if residual in ['chroma-dc','all-ac']:
            for c in range(2):b.u(1);b.u(c^negative);b.u(1)
        if residual=='all-ac':
            for c in range(2):
                for i in range(4):b.u(1,2);b.u(((i+c)%2)^negative);b.u(1)
    return b.nal(0x01)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    variants=[('zero',0,0)]+[(m,p,s) for m in ['luma-ac','chroma-dc','all-ac'] for p in [0,1] for s in [0,1]]
    with tempfile.TemporaryDirectory(prefix='fvid-residual-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for spatial in [False,True]:
                    for inference in [False,True]:
                        for deblock in [0,1,2]:
                            for mode,pos,negative in variants:
                                order=[True,False] if reverse else [False,True];frames=[]
                                for number in [0,1]:
                                    for i,bottom in enumerate(order):
                                        index=number*4+i;n=intra(bottom,depth,number,index==0,index);frames.append((index,index==0,len(n).to_bytes(4,'big')+n))
                                for i,bottom in enumerate(order):
                                    nals=[p_field(bottom,8+i,a,deblock) for a in [0,1]];frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                for i,bottom in enumerate(order):
                                    n=b_field(bottom,6+i,spatial,mode,pos,negative,deblock);frames.append((6+i,False,len(n).to_bytes(4,'big')+n))
                                config=configuration(depth,direct8=inference)
                                name=f'avc-field-b-residual-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+f'-{mode}-pos{pos}-sign{negative}-filter{deblock}-infer{int(inference)}'
                                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
                                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-field-b-residual-generated.json').write_text(json.dumps(dict(generator='owned signed CAVLC B direct residual writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
