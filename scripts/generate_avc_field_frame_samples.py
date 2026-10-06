#!/usr/bin/env python3
"""Owned native reference fields followed by full-frame P/B prediction.
Explicit generation uses local JM; ordinary tests consume saved owned artifacts.
"""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import Writer,configuration as paff_config
from generate_avc_mbaff_direct_samples import config
from generate_avc_field_reference_samples import intra
from avc_fixture_mp4 import mux,annexb

def p_field(bottom,poc,address,motion,partial=False,mixed=False,number=1):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(number,4);b.u(1);b.u(int(bottom));b.u(poc,4)
    b.u(0);b.u(0)
    if partial:b.u(1);b.ue(1);b.ue(1);b.ue(0)
    elif mixed:b.u(1);b.ue(4);b.ue(2);b.ue(6);b.ue(1);b.ue(0)
    else:b.u(0)
    b.se(0);b.ue(1);b.ue(0);b.ue(0);b.se(((8 if not bottom else -4)*(address+1)) if motion else 0);b.se((4 if not bottom else -2) if motion else 0);b.ue(0)
    return b.nal(0x41)

def full_frame(paff,kind,skip,motion,number=None,poc=None):
    is_b=kind!='p';nals=[]
    for first in range(4 if paff else 2):
        b=Writer();b.ue(first);b.ue(1 if is_b else 0);b.ue(0);b.u((2 if is_b else 1) if number is None else number,4);b.u(0);b.u((4 if is_b else 2) if poc is None else poc,4)
        if is_b:b.u(int(kind=='spatial'))
        b.u(0);b.u(0)
        if is_b:b.u(0)
        else:b.u(0)
        b.se(0);b.ue(1)
        for local in range(1 if paff else 2):
            if skip:
                # One run covers both MBAFF pair members; their field flag is inferred.
                if local==0:b.ue(1 if paff else 2)
                continue
            b.ue(0)
            if not paff and local==0:b.u(0)
            b.ue(0)
            if not is_b:b.se(8 if motion else 0);b.se(4 if motion else 0)
            b.ue(0)
        nals.append(b.nal(0x01 if is_b else 0x41))
    return nals

def emit(root,d,cfg,jm,name,configuration,frames,count):
    coded=d/'sample.264';oracle=d/'sample.yuv';coded.write_bytes(annexb(configuration,frames))
    subprocess.run([str(jm),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
    pixels=oracle.read_bytes();assert len(pixels)==count,(name,len(pixels),count)
    data=mux(configuration,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
    return dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest())

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-field-frame-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,paff,reverse,long_term,kind,skip,motion in itertools.product([8,10],[False,True],[False,True],[False,True],['p','temporal','spatial'],[False,True],[False,True]):
            configuration=paff_config(depth) if paff else config(depth,False,width_mbs=2);order=[True,False] if reverse else [False,True];frames=[]
            for i,bottom in enumerate(order):frames.append((i,i==0,packet([intra(bottom,depth,0,i==0,i,long_term=long_term)])))
            if kind!='p':
                for i,bottom in enumerate(order):frames.append((8+i,False,packet([p_field(bottom,8+i,a,motion) for a in [0,1]])))
            frames.append((2 if kind=='p' else 4,False,packet(full_frame(paff,kind,skip,motion))))
            name='avc-field-frame-'+('paff' if paff else 'mbaff')+f'-{depth}bit-'+('bottom' if reverse else 'top')+('-long' if long_term else '-short')+f'-{kind}'+('-skip' if skip else '-coded')+('-motion' if motion else '-zero')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration,frames,(3072 if kind=='p' else 4608)*(2 if depth==10 else 1)))
        # Three successive migrations: native fields -> frame -> fields -> frame.
        for depth,paff,reverse,spatial,skip,motion in itertools.product([8,10],[False,True],[False,True],[False,True],[False,True],[False,True]):
            configuration=paff_config(depth) if paff else config(depth,False,width_mbs=2);order=[True,False] if reverse else [False,True];frames=[]
            for i,bottom in enumerate(order):frames.append((i,i==0,packet([intra(bottom,depth,0,i==0,i)])))
            frames.append((2,False,packet(full_frame(paff,'p',False,motion))))
            for i,bottom in enumerate(order):frames.append((8+i,False,packet([p_field(bottom,8+i,a,motion,number=2) for a in [0,1]])))
            frames.append((4,False,packet(full_frame(paff,'spatial' if spatial else 'temporal',skip,motion,number=3))))
            name='avc-field-frame-roundtrip-'+('paff' if paff else 'mbaff')+f'-{depth}bit-'+('bottom' if reverse else 'top')+('-spatial' if spatial else '-temporal')+('-skip' if skip else '-coded')+('-motion' if motion else '-zero')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration,frames,6144*(2 if depth==10 else 1)))
        # Retain compact refusals for storage shapes not represented by frame DPB.
        for mode in ['partial','mixed']:
            frames=[];configuration=paff_config(8)
            for i,bottom in enumerate([False,True]):frames.append((i,i==0,packet([intra(bottom,8,0,i==0,i)])))
            for i,bottom in enumerate([False,True]):
                # Repeat the same picture marking in every slice header.
                frames.append((8+i,False,packet([p_field(bottom,8+i,a,True,partial=mode=='partial' and i==0,mixed=mode=='mixed' and i==0) for a in [0,1]])))
            frames.append((12,False,packet(full_frame(True,'p',False,True,number=2,poc=12))))
            records.append(emit(root,d,cfg,args.jm_decoder,'avc-field-frame-'+mode+'-refusal',configuration,frames,4608))
    (root/'avc-field-frame-generated.json').write_text(json.dumps(dict(generator='owned native field references to full frame prediction',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
