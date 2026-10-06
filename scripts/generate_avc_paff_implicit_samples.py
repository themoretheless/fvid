#!/usr/bin/env python3
"""Owned PAFF implicit bipred with asymmetric POCs and average controls."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_paff_weight_samples import configuration,slice
from avc_fixture_mp4 import mux,annexb

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-implicit-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,implicit,selected,target,mode,filter,aso in itertools.product([8,10],[False,True],[0,1],[2,6],['16x16','16x8','8x16','8x8'],range(3),[False,True]):
            bipred=2 if implicit else 0;config=configuration(depth,bipred);frames=[];pocs=(0,8,target)
            for index,pts in enumerate(pocs):
                ns=[slice(a,index,depth,True,selected,mode,filter,bipred,pocs) for a in ([3,1,2,0] if aso else range(4))];frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            name=f'avc-paff-implicit-{depth}bit-'+('implicit' if implicit else 'average')+f'-ref{selected}-poc{target}-{mode}-filter{filter}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-implicit-generated.json').write_text(json.dumps(dict(generator='owned PAFF asymmetric implicit weighted partitions',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
