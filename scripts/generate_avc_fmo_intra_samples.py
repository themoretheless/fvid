#!/usr/bin/env python3
"""Owned FMO PCM/I16 DC prediction and residual/filter fixtures; explicit JM oracle."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from generate_avc_fmo_samples import configuration
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def slice_nal(addresses,map_type,deblock,residual):
    b=Writer();b.ue(addresses[0]);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if map_type in [3,4,5]:b.u(2,3)
    done={}
    for index,address in enumerate(addresses):
        if index==0:
            b.ue(25);b.align();b.bits.extend(pcm_samples(8,address//2,address%2));done[address]=16
        else:
            b.ue(3);b.ue(0);b.se(0)
            neighbours=[done[n] for n in [address-2,address-1 if address%2 else -1] if n in done]
            nc=(sum(neighbours)+1)//2 if len(neighbours)==2 else neighbours[0] if neighbours else 0
            assert nc in [0,8,16]
            if residual:
                b.u(1,6 if nc>=8 else 2);b.u(0);b.u(1) # one positive trailing one, zero total_zeros
            else:b.u(3,6) if nc>=8 else b.u(1)
            done[address]=0
    return b.nal(0x65)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-fmo-intra-') as tmp:
        directory=Path(tmp);cfg=directory/'decoder.cfg';cfg.write_text('')
        for map_type,direction,reverse,deblock,residual in [(m,d,r,f,n) for m in range(7) for d in ([False,True] if m in [3,4,5] else [False]) for r in [False,True] for f in [0,1,2] for n in [False,True]]:
            config=configuration(map_type,direction)
            maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]}
            nals=[slice_nal([i for i,g in enumerate(maps[map_type]) if g==group],map_type,deblock,residual) for group in ([1,0] if reverse else [0,1])]
            frames=[(0,True,b''.join(len(n).to_bytes(4,'big')+n for n in nals))]
            name=f'avc-fmo-intra-type{map_type}-dir{int(direction)}-filter{deblock}'+('-dc1' if residual else '-dc0')+('-aso' if reverse else '')
            coded=directory/(name+'.264');oracle=directory/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
            pixels=oracle.read_bytes();assert len(pixels)==1536
            data=mux(config,frames,32,32,25);(output/(name+'.mp4')).write_bytes(data);(output/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-fmo-intra-generated.json').write_text(json.dumps(dict(generator='owned PCM/I16 DC CAVLC writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
