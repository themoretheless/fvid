#!/usr/bin/env python3
"""Owned 3–8 group FMO Extended I/P/B streams; JM generation only."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_fmo_inter_samples import slice_nal
from avc_fixture_mp4 import mux,annexb

def mapping(kind,groups):
    if kind==0:return [i%groups for i in range(16)]
    if kind==1:return [(i%4+(i//4*groups)//2)%groups for i in range(16)]
    if kind==2:return [i if i<groups-1 else groups-1 for i in range(16)]
    return [(i%4*3+i//4)%groups for i in range(16)]

def configuration(kind,groups,map):
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(3);b.ue(3);b.u(1);b.u(1);b.u(0);b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(groups-1);b.ue(kind)
    if kind==0:
        for _ in range(groups):b.ue(0)
    elif kind==2:
        for index in range(groups-1):b.ue(index);b.ue(index)
    elif kind==6:
        b.ue(15)
        for group in map:b.u(group,(groups-1).bit_length())
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-fmo-groups-') as tmp:
        directory=Path(tmp);cfg=directory/'decoder.cfg';cfg.write_text('')
        for groups,kind,reverse,filter in [(g,k,r,f) for g in range(3,9) for k in [0,1,2,6] for r in [False,True] for f in [0,1,2]]:
            map=mapping(kind,groups);assert set(map)==set(range(groups));config=configuration(kind,groups,map)
            frames=[]
            for picture,pts in [('I',0),('P',2),('B',1)]:
                order=list(range(groups));order=order[::-1] if reverse else order
                nals=[slice_nal(picture,g,addresses=[i for i,v in enumerate(map) if v==g],map_type=kind,width_mbs=4,deblock=filter,qp=26 if filter==1 else 50) for g in order]
                frames.append((pts,picture=='I',b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            name=f'avc-fmo-groups{groups}-type{kind}-filter{filter}'+('-aso' if reverse else '')
            coded=directory/(name+'.264');oracle=directory/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
            pixels=oracle.read_bytes();assert len(pixels)==18432;data=mux(config,frames,64,64,25)
            (output/(name+'.mp4')).write_bytes(data);(output/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-fmo-multigroup-generated.json').write_text(json.dumps(dict(generator='owned 3-8 group headers PCM and CAVLC motion/skip writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
