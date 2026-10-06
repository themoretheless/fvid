#!/usr/bin/env python3
"""Owned Extended-profile MBAFF FMO I/P/B motion and signed residual fixtures, explicit separate JM oracle."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def configuration():
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0)
    b.ue(1);b.ue(1);b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(1);b.ue(6);b.ue(3)
    for group in [0,1,0,1]:b.u(group)
    b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def slice_nal(group,fields,deblock):
    b=Writer();b.ue(group);b.ue(2);b.ue(0);b.u(0,4);b.u(0);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for pair in [group,group+2]:
        for parity in [0,1]:
            if parity==0:b.u(int(fields[pair]))
            b.ue(25);b.align();b.bits.extend(pcm_samples(8,pair//2*2+parity,pair%2))
    return b.nal(0x65)

def inter_nal(kind,group,fields,deblock,mode):
    b=Writer();b.ue(group);b.ue(0 if kind=='P' else 1);b.ue(0);b.u(1 if kind=='P' else 2,4);b.u(0);b.u(4 if kind=='P' else 2,4)
    if kind=='B':b.u(0)
    b.u(0);b.u(0)
    if kind=='B':b.u(0)
    if kind=='P':b.u(0)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for pair in [group,group+2]:
        for parity in [0,1]:
            b.ue(0)
            if parity==0:b.u(int(fields[pair]))
            b.ue(0 if kind=='P' else 1)
            if fields[pair]:b.u(0)
            b.se(8+group*4 if parity==0 or fields[pair] else 0);b.se(4 if parity==0 or fields[pair] else 0);b.ue({'luma':11,'chroma':1,'combined':12}[mode]);b.se(0)
            if mode in ['luma','combined']:
                for i in range(16):b.u(1,2);b.u((i+parity)%2);b.u(1)
            if mode in ['chroma','combined']:
                for c in range(2):b.u(1);b.u((c+parity)%2);b.u(1)
            if mode=='combined':
                for c in range(2):
                    for i in range(4):b.u(1,2);b.u((i+c+parity)%2);b.u(1)
    return b.nal(0x41 if kind=='P' else 0x01)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    out=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';config=configuration();records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-mbaff-fmo-residual-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for topology,fields in [('frame',[False]*4),('field',[True]*4),('mixed',[False,True,True,False])]:
            for reverse in [False,True]:
                for deblock,mode in [(f,m) for f in [0,1,2] for m in ["luma","chroma","combined"]]:
                    nals=[slice_nal(g,fields,deblock) for g in ([1,0] if reverse else [0,1])]
                    frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in nals))]
                    for kind,pts in [('P',2),('B',1)]:
                        nals=[inter_nal(kind,g,fields,deblock,mode) for g in ([1,0] if reverse else [0,1])]
                        frames.append((pts,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                    name=f'avc-mbaff-fmo-residual-{mode}-{topology}-filter{deblock}'+('-aso' if reverse else '')
                    coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                    subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True)
                    pixels=oracle.read_bytes();assert len(pixels)==9216
                    data=mux(config,frames,32,64,25);(out/(name+'.mp4')).write_bytes(data);(out/(name+'.yuv')).write_bytes(pixels)
                    records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out/'avc-mbaff-fmo-residual-generated.json').write_text(json.dumps(dict(generator='owned Extended-profile MBAFF FMO I/P/B signed inter residual writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
