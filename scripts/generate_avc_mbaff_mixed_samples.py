#!/usr/bin/env python3
"""Owned MBAFF mixed I/P/B slices; JM oracle, no private media or external encoder."""
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
    with tempfile.TemporaryDirectory(prefix='fvid-mbaff-mixed-slices-') as temporary:
        directory=Path(temporary); cfg=directory/'decoder.cfg'; cfg.write_text('')
        for depth,cabac in [(d,c) for d in [8,10] for c in [False,True]]:
            for types in ['IP','PI','IB','BI','PB','BP']:
                for explicit in ([False] if types in ['IP','PI'] else [False,True]):
                    for left_field in [False,True]:
                        for deblock in [0,1,2]:
                            configuration=config(depth,explicit,cabac,width_mbs=2)
                            frames=[]; b_mixed=types not in ['IP','PI']
                            timeline=[0,4,2,1,3] if b_mixed else [0,1,2]
                            for index,pts in enumerate(timeline):
                                nals=[]
                                for pair in range(2):
                                    if b_mixed:
                                        kind='I' if index==0 else 'P' if index==1 else ('Bexplicit' if types[pair]=='B' else types[pair]) if index==2 else 'Bdirect' if index==3 else 'Bexplicit'
                                        frame_num=[0,1,2,3,3][index]
                                        field=(left_field if pair==0 else not left_field) ^ (index>=3)
                                        reference=index<3
                                    else:
                                        kind='I' if index==0 else types[pair] if index==1 else 'P'
                                        frame_num=index; field=left_field if pair==0 else not left_field; reference=True
                                    nals.append(picture(depth,kind,frame_num,pts*2,field,reference,explicit,cabac=cabac,first_mb=pair,idr=index==0,deblock=deblock,qp=50 if kind!='I' and index==(2 if b_mixed else 1) else 26,l0_to_idr=b_mixed and index==2 and kind=='P',filter_offsets=(12,12) if b_mixed and index==2 else (0,0)))
                                payload=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                frames.append((pts,index==0,payload))
                            name='avc-mbaff-mixed-'+types.lower()+'-'+('field-frame' if left_field else 'frame-field')+'-filter'+str(deblock)+('-explicit' if explicit else '')+('-high10' if depth==10 else '')+('-cabac' if cabac else '')
                            coded=directory/(name+'.264'); oracle=directory/(name+'.yuv')
                            coded.write_bytes(annexb(configuration,frames))
                            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
                            pixels=oracle.read_bytes(); assert len(pixels)==len(timeline)*32*32*3//2*(2 if depth>8 else 1)
                            data=mux(configuration,frames,32,32,25)
                            (output/(name+'.mp4')).write_bytes(data); (output/(name+'.yuv')).write_bytes(pixels)
                            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-mbaff-mixed-generated.json').write_text(json.dumps(dict(generator='owned header/PCM/CAVLC/CABAC writer; mixed slice types',jm_decoder_sha256=hashlib.sha256(args.jm_decoder.read_bytes()).hexdigest(),fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
