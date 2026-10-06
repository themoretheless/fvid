#!/usr/bin/env python3
"""Owned Extended-profile MBAFF FMO PCM/I16 DC residual fixtures, explicit separate JM oracle."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def configuration():
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(0);b.ue(2);b.ue(1);b.u(0)
    b.ue(1);b.ue(1);b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(1);b.ue(6);b.ue(3)
    for group in [0,1,0,1]:b.u(group)
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def slice_nal(group,fields,deblock,residual,luma_mode,chroma_mode):
    b=Writer();b.ue(group);b.ue(2);b.ue(0);b.u(0,4);b.u(0);b.ue(0);b.u(0);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for pair in [group,group+2]:
        for parity in [0,1]:
            if parity==0:b.u(int(fields[pair]))
            if pair<2:
                b.ue(25);b.align();b.bits.extend(pcm_samples(8,pair//2*2+parity,pair%2))
            else:
                b.ue(luma_mode+1);b.ue(chroma_mode);b.se(0)
                nc16 = parity==0 or fields[pair]
                if residual:
                    b.u(1,6 if nc16 else 2);b.u(0);b.u(1)
                else:b.u(3,6) if nc16 else b.u(1)
    return b.nal(0x65)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';config=configuration();records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-mbaff-fmo-intra-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for topology,fields in [('frame',[False]*4),('field',[True]*4),('mixed',[False,True,True,False])]:
            for reverse in [False,True]:
                for deblock,residual,luma_mode,chroma_mode in [(f,r,l,c) for f in [0,1,2] for r in [False,True] for l in [0,2] for c in [0,2]]:
                    nals=[slice_nal(g,fields,deblock,residual,luma_mode,chroma_mode) for g in ([1,0] if reverse else [0,1])]
                    frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in nals))]
                    name=f'avc-mbaff-fmo-intra-{topology}-filter{deblock}-dc{int(residual)}-l{luma_mode}-c{chroma_mode}'+('-aso' if reverse else '')
                    coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                    subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True)
                    pixels=oracle.read_bytes();assert len(pixels)==3072
                    data=mux(config,frames,32,64,25);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                    records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-mbaff-fmo-intra-generated.json').write_text(json.dumps(dict(generator='owned Extended-profile MBAFF FMO PCM/I16 DC writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
