#!/usr/bin/env python3
"""Owned nonzero full-frame P motion followed by reordered B direct fields.
Generation explicitly requires a local JM decoder; ordinary tests read artifacts.
"""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import Writer,configuration as paff_config
from generate_avc_mbaff_direct_samples import config
from generate_avc_frame_field_samples import frame
from avc_fixture_mp4 import mux,annexb

def p_frame(mode,motion):
    paff=mode=="paff"
    nals=[]
    for first in range(4 if paff else 2):
        b=Writer();b.ue(first);b.ue(0);b.ue(0);b.u(1,4);b.u(0);b.u(8,4)
        b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
        field=mode=="field" or (mode=="mixed" and first==1)
        for local in range(1 if paff else 2):
            b.ue(0)
            if not paff and local==0:b.u(int(field))
            b.ue(0)
            if field:b.ue(0)
            b.se(8 if motion else 0);b.se(4 if motion else 0);b.ue(0)
        nals.append(b.nal(0x41))
    return nals

def b_field(bottom,poc,spatial,skip,address):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(poc,4)
    b.u(int(spatial));b.u(1);b.ue(1);b.ue(1);b.u(0);b.u(0);b.se(0);b.ue(1)
    b.ue(1 if skip else 0)
    if not skip:b.ue(0);b.ue(0)
    return b.nal(0x01)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-frame-field-direct-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,mode,reverse,spatial,skip,motion in itertools.product([8,10],["paff","mbaff","field","mixed"],[False,True],[False,True],[False,True],[False,True]):
            paff=mode=="paff"
            configuration=paff_config(depth) if paff else config(depth,False,width_mbs=2)
            packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
            frames=[(0,True,packet([frame(depth,paff=paff)])),(8,False,packet(p_frame(mode,motion)))]
            for i,bottom in enumerate([True,False] if reverse else [False,True]):
                frames.append((2+i,False,packet([b_field(bottom,2+i,spatial,skip,a) for a in [0,1]])))
            name='avc-frame-field-direct-'+mode+f'-{depth}bit-'+('bottom' if reverse else 'top')+('-spatial' if spatial else '-temporal')+('-skip' if skip else '-coded')+('-motion' if motion else '-zero')
            coded=d/'sample.264';oracle=d/'sample.yuv';coded.write_bytes(annexb(configuration,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels))
            data=mux(configuration,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-frame-field-direct-generated.json').write_text(json.dumps(dict(generator='owned full-frame P to reordered direct B fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
