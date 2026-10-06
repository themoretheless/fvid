#!/usr/bin/env python3
"""Owned PAFF full-frame FMO PCM/P/B streams covering map types 0..6."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_fmo_samples import configuration
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def slice(addresses,kind,index,depth,skip,spatial,filter,residual=None):
    b=Writer();b.ue(addresses[0]);b.ue([2,0,1][index]);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u([0,4,2][index],4)
    if index==2:b.u(int(spatial))
    if index!=0:
        b.u(0);b.u(0)
        if index==2:b.u(0)
    if index==0:b.u(0);b.u(0)
    elif index==1:b.u(0)
    b.se(24);b.ue(filter)
    if filter!=1:b.se(6);b.se(6)
    if kind in [3,4,5]:b.u(2,3)
    if index==0:
        for a in addresses:b.ue(25);b.align();b.bits.extend(pcm_samples(depth,a//2,a%2))
    elif residual is not None:
        b.ue(0);b.ue(0 if index==1 else 1);b.se(4);b.se(4 if index==1 else 0)
        b.ue({'none':0,'luma-ac':11,'chroma-dc':1,'all-ac':12}[residual])
        if residual!='none':b.se(0)
        if residual in ['luma-ac','all-ac']:
            for block in range(16):b.u(1,2);b.u(block%2);b.u(3,3)
        if residual in ['chroma-dc','all-ac']:
            for component in range(2):b.u(1);b.u(component);b.u(1)
        if residual=='all-ac':
            for component in range(2):
                for block in range(4):b.u(1,2);b.u((block+component)%2);b.u(1)
        if len(addresses)>1:b.ue(len(addresses)-1)
    elif skip:b.ue(len(addresses))
    else:
        for a in addresses:
            b.ue(0);b.ue(0)
            if index==1:b.se(4);b.se(4)
            b.ue(0)
    return b.nal(0x65 if index==0 else 0x41 if index==1 else 0x01)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-fmo-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,kind in itertools.product([8,10],range(7)):
            for direction in [False,True] if kind in [3,4,5] else [False]:
                maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]};mbmap=[maps[kind][(a//4)*2+a%2] for a in range(8)]
                for skip,filter,aso in itertools.product([False,True],range(3),[False,True]):
                    for spatial in [False,True]:
                        config=configuration(depth,kind,direction);frames=[]
                        for index,pts in enumerate([0,4,2]):
                            ns=[slice([a for a,g in enumerate(mbmap) if g==group],kind,index,depth,skip if index==2 else False,spatial,filter) for group in ([1,0] if aso else [0,1])]
                            frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
                        name=f'avc-paff-fmo-{depth}bit-type{kind}-dir{int(direction)}-'+('spatial' if spatial else 'temporal')+'-'+('skip' if skip else 'direct')+f'-filter{filter}'+('-aso' if aso else '')
                        coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                        pixels=oracle.read_bytes();assert len(pixels)==9216*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,64,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-fmo-generated.json').write_text(json.dumps(dict(generator='owned PAFF FMO full-frame writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
