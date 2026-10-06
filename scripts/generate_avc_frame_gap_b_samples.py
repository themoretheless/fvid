#!/usr/bin/env python3
"""Owned MBAFF B gap lists and post-gap reference history; explicit JM oracle."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from avc_fixture_mp4 import mux, annexb
from generate_avc_mbaff_direct_samples import config, picture

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,required=True)
    args=parser.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-b-gap-') as tmp:
        directory=Path(tmp); cfg=directory/'decoder.cfg'; cfg.write_text('')
        for depth in [8,10]:
            for cabac in [False,True]:
                for field in [False,True]:
                    for poc_type, kind in [(p,k) for p in [1,2] for k in ['Bexplicit','Bdirect']]:
                        configuration=config(depth,False,cabac,gaps_allowed=True,poc_type=poc_type)
                        nals=[picture(depth,'I',0,0,False,True,False,cabac=cabac,poc_type=poc_type),
                              picture(depth,kind,3,4,field,False,False,cabac=cabac,l0_to_idr=True,l1_to_idr=True,active_l0=3,poc_type=poc_type),
                              picture(depth,'P',3,6,field,True,False,cabac=cabac,l0_to_idr=True,poc_type=poc_type)]
                        frames=[(i,i==0,len(n).to_bytes(4,'big')+n) for i,n in enumerate(nals)]
                        name=('avc-frame-num-gap-b-direct-' if kind=='Bdirect' else 'avc-frame-num-gap-b-')+f'poc{poc_type}-'+('field' if field else 'frame')+('-high10' if depth==10 else '')+'-'+('cabac' if cabac else 'cavlc')
                        coded=directory/(name+'.264'); oracle=directory/(name+'.yuv')
                        coded.write_bytes(annexb(configuration,frames))
                        subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
                        pixels=oracle.read_bytes(); assert len(pixels)==3*16*32*3//2*(2 if depth>8 else 1)
                        data=mux(configuration,frames,16,32,25)
                        (output/(name+'.mp4')).write_bytes(data); (output/(name+'.yuv')).write_bytes(pixels)
                        records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-frame-num-gap-b-generated.json').write_text(json.dumps(dict(generator='owned B/P slice and PCM writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
