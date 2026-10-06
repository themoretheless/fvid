#!/usr/bin/env python3
"""Owned PAFF FMO signed P/B luma/chroma residuals and no-residual controls."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_paff_fmo_samples import slice
from generate_avc_field_fmo_samples import configuration
from avc_fixture_mp4 import mux,annexb

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-fmo-residual-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,kind in itertools.product([8,10],range(7)):
            for direction in [False,True] if kind in [3,4,5] else [False]:
                maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]};mbmap=[maps[kind][(a//4)*2+a%2] for a in range(8)]
                for mode,filter,aso in itertools.product(['none','luma-ac','chroma-dc','all-ac'],range(3),[False,True]):
                    config=configuration(depth,kind,direction);frames=[]
                    for index,pts in enumerate([0,4,2]):
                        ns=[slice([a for a,g in enumerate(mbmap) if g==group],kind,index,depth,False,False,filter,residual=None if index==0 else mode) for group in ([1,0] if aso else [0,1])]
                        frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
                    name=f'avc-paff-fmo-residual-{depth}bit-type{kind}-dir{int(direction)}-{mode}-filter{filter}'+('-aso' if aso else '')
                    coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                    pixels=oracle.read_bytes();assert len(pixels)==9216*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,64,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-fmo-residual-generated.json').write_text(json.dumps(dict(generator='owned PAFF FMO signed residual writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
