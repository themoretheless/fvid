#!/usr/bin/env python3
"""Owned PAFF CAVLC P and reordered direct B frames, joined or ASO."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration,Writer
from generate_avc_paff_intra_samples import frame_slice
from avc_fixture_mp4 import mux,annexb

def inter(address,b_frame,skip,spatial,filter,joined,poc):
    b=Writer();b.ue(address);b.ue(1 if b_frame else 0);b.ue(0);b.u(2 if b_frame else 1,4);b.u(0);b.u(poc,4)
    if b_frame:b.u(int(spatial))
    b.u(0);b.u(0)
    if b_frame:b.u(0)
    else:b.u(0)
    b.se(24);b.ue(filter)
    if filter!=1:b.se(6);b.se(6)
    if skip:b.ue(4 if joined else 1)
    else:
        for at in range(4) if joined else [address]:
            b.ue(0);b.ue(0)
            if not b_frame:b.se(0 if joined and at else 4);b.se(0 if joined and at else 4)
            b.ue(0)
    return b.nal(0x01 if b_frame else 0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-cavlc-inter-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for b_frame,depth,skip,filter,layout in itertools.product([False,True],[8,10],[False,True],range(3),['normal','aso','joined']):
            for spatial in [False,True] if b_frame else [False]:
                joined=layout=='joined';config=configuration(depth);frames=[]
                for index,pts in enumerate([0,4,2] if b_frame else [0,2]):
                    addresses=[3,1,2,0] if layout=='aso' else range(4)
                    ns=[frame_slice(a,'i16-positive' if a%2==0 else 'i16-negative',filter) for a in addresses] if index==0 else [inter(a,index==2,skip if index==2 or not b_frame else False,spatial,filter,joined,pts) for a in ([0] if joined else addresses)]
                    frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
                name=f'avc-paff-cavlc-'+('b-' if b_frame else 'p-')+f'{depth}bit-'+(('spatial-' if spatial else 'temporal-') if b_frame else '')+('skip' if skip else 'coded')+f'-filter{filter}-{layout}'
                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                pixels=oracle.read_bytes();assert len(pixels)==1536*len(frames)*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-cavlc-inter-generated.json').write_text(json.dumps(dict(generator='owned PAFF CAVLC inter writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
