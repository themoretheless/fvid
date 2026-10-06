#!/usr/bin/env python3
"""Owned PAFF CAVLC weighted P / two-reference explicit Bi B partitions."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration as cabac_configuration
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def configuration(depth,bipred=1):
    config=cabac_configuration(depth);size=int.from_bytes(config[6:8],'big');sps=config[8:8+size]
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(1);b.u(bipred,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return config[:6]+len(sps).to_bytes(2,'big')+sps+b'\x01'+len(pps).to_bytes(2,'big')+pps

def slice(address,index,depth,identity,selected,mode,filter,bipred=1,pocs=(0,4,2)):
    b=Writer();b.ue(address);b.ue([2,0,1][index]);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u(pocs[index],4)
    if index==2:b.u(0)
    if index:
        b.u(int(index==2))
        if index==2:b.ue(1);b.ue(1)
        b.u(0)
        if index==2:b.u(0)
        if index==1 or bipred==1:
            b.ue(2);b.ue(1)
            tables=[[(3,5,[(1,-7),(3,8)]),(-2,90,[(0,24),(-1,48)])],[(5,-24,[(3,7),(1,-8)]),(1,9,[(2,-10),(4,3)])]]
            for table in tables[:2 if index==2 else 1]:
                for weight,offset,chroma in table[:2 if index==2 else 1]:
                    b.u(int(not identity))
                    if not identity:b.se(weight);b.se(offset)
                    b.u(int(not identity))
                    if not identity:
                        for w,o in chroma:b.se(w);b.se(o)
    if index==0:b.u(0);b.u(0)
    elif index==1:b.u(0)
    b.se(24);b.ue(filter)
    if filter!=1:b.se(6);b.se(6)
    if index==0:b.ue(25);b.align();b.bits.extend(pcm_samples(depth,address//2,address%2))
    else:
        codes={'16x16':(0,3,1),'16x8':(1,20,2),'8x16':(2,21,2),'8x8':(3,22,4)};pcode,bcode,count=codes[mode];b.ue(0);b.ue(bcode if index==2 else pcode)
        if mode=='8x8':
            for _ in range(4):b.ue(3 if index==2 else 0)
        if index==2:
            for _ in range(2):
                for _ in range(count):b.u(1-selected)
        for list in range(2 if index==2 else 1):
            for _ in range(count):b.se(-4 if list else 4);b.se(0 if index==2 else 4)
        b.ue(0)
    return b.nal(0x65 if index==0 else 0x41 if index==1 else 0x01)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-weight-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,identity,selected,mode,filter,aso in itertools.product([8,10],[False,True],[0,1],['16x16','16x8','8x16','8x8'],range(3),[False,True]):
            config=configuration(depth);frames=[]
            for index,pts in enumerate([0,4,2]):
                ns=[slice(a,index,depth,identity,selected,mode,filter) for a in ([3,1,2,0] if aso else range(4))];frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            name=f'avc-paff-weight-{depth}bit-'+('identity' if identity else 'weighted')+f'-ref{selected}-{mode}-filter{filter}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-weight-generated.json').write_text(json.dumps(dict(generator='owned PAFF weighted multireference partitions',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
