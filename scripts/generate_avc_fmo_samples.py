#!/usr/bin/env python3
"""Owned progressive I_PCM FMO slices; fixture generation is separate from tests."""
import argparse, subprocess, tempfile, hashlib, json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, pcm_samples
from avc_fixture_mp4 import mux, annexb

def configuration(map_type=6,direction=False,poc_type=2,profile=66,max_refs=1):
    b=Writer();b.u(profile,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(0);b.ue(poc_type);
    if poc_type==0:b.ue(0)
    b.ue(max_refs);b.u(0);b.ue(1);b.ue(1);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(1);b.ue(map_type)
    if map_type==0:b.ue(0);b.ue(0)
    elif map_type==2:b.ue(0);b.ue(0)
    elif map_type in [3,4,5]:b.u(int(direction));b.ue(0)
    elif map_type==6:
        b.ue(3)
        for group in [0,1,0,1]:b.u(group)
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,profile,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def slice_nal(addresses,map_type=6):
    b=Writer();b.ue(addresses[0]);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0);b.u(0);b.se(0);b.ue(1)
    if map_type in [3,4,5]:b.u(2,3)
    for address in addresses:
        b.ue(25);b.align();b.bits.extend(pcm_samples(8,address//2,address%2))
    return b.nal(0x65)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';config=configuration();records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-fmo-') as tmp:
        directory=Path(tmp);cfg=directory/'decoder.cfg';cfg.write_text('')
        for map_type,direction,reverse in [(m,d,r) for m in range(7) for d in ([False,True] if m in [3,4,5] else [False]) for r in [False,True]]:
            config=configuration(map_type,direction)
            maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]}
            mapping=maps[map_type]
            nals=[slice_nal([i for i,g in enumerate(mapping) if g==group],map_type) for group in ([1,0] if reverse else [0,1])]
            frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in nals))]
            name=('avc-fmo-explicit-pcm' if map_type==6 else f'avc-fmo-type{map_type}-dir{int(direction)}-pcm')+('-aso' if reverse else '')
            coded=directory/(name+'.264');oracle=directory/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
            pixels=oracle.read_bytes();assert len(pixels)==1536
            data=mux(config,frames,32,32,25)
            (output/(name+'.mp4')).write_bytes(data);(output/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-fmo-generated.json').write_text(json.dumps(dict(generator='owned explicit FMO headers and PCM writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
