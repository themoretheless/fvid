#!/usr/bin/env python3
"""Owned PAFF CABAC weighted multireference P/B partition streams."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration
from generate_avc_mbaff_direct_samples import Writer,CabacWriter,pcm_samples
from avc_fixture_mp4 import mux,annexb

def tree(c,code,base):
    bits=0
    for index,symbol in enumerate(code):
        ctx=base if index==0 else base+3 if index==1 else base+4 if index==2 and bits&1 else base+5
        if base==36:ctx=36 if index==0 else 37 if index==1 else 38 if index==2 and bits&1 else 39
        c.decision(ctx,int(symbol));bits=(bits<<1)|int(symbol)

def slice(address,index,depth,identity,selected,mode,filter,init,bipred=1,pocs=(0,4,2)):
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
    if index:b.ue(init)
    b.se(24);b.ue(filter)
    if filter!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init if index else -1,50)
    if index==0:
        c.decision(3,1);b.bits.extend(c.finish());b.align();b.bits.extend(pcm_samples(depth,address//2,address%2));c.low=0;c.range=510;c.width=9
    else:
        c.decision(24 if index==2 else 11,0)
        if index==2:
            tree(c,{'16x16':'110000','16x8':'1111000','8x16':'1111001','8x8':'111111'}[mode],27)
            if mode=='8x8':
                for _ in range(4):tree(c,'11000',36) # B_Bi_8x8
        else:
            c.decision(14,0);c.decision(15,int(mode in ['16x8','8x16']))
            if mode in ['16x8','8x16']:c.decision(17,int(mode=='16x8'))
            else:c.decision(16,int(mode=='8x8'))
            if mode=='8x8':
                for _ in range(4):c.decision(21,1)
        origins={'16x16':[(0,0)],'16x8':[(0,0),(0,2)],'8x16':[(0,0),(2,0)],'8x8':[(0,0),(2,0),(0,2),(2,2)]}[mode]
        width,height={'16x16':(4,4),'16x8':(4,2),'8x16':(2,4),'8x8':(2,2)}[mode]
        if index==2:
            for list in range(2):
                references=[None]*16
                for x,y in origins:
                    left=references[y*4+x-1] if x else None;top=references[(y-1)*4+x] if y else None;inc=int(left is not None and left>0)+2*int(top is not None and top>0)
                    c.decision(54+inc,selected)
                    if selected:c.decision(58,0)
                    for dy in range(height):
                        for dx in range(width):references[(y+dy)*4+x+dx]=selected
        for list in range(2 if index==2 else 1):
            magnitudes=[[0,0] for _ in range(16)]
            for x,y in origins:
                vector=[-4 if list else 4,0 if index==2 else 4]
                for component,value in enumerate(vector):
                    neighbour=(magnitudes[y*4+x-1][component] if x else 0)+(magnitudes[(y-1)*4+x][component] if y else 0);c.mvd(component,value,neighbour)
                for dy in range(height):
                    for dx in range(width):magnitudes[(y+dy)*4+x+dx]=[abs(v) for v in vector]
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x65 if index==0 else 0x41 if index==1 else 0x01,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-paff-cabac-weight-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,identity,selected,mode,filter,aso,init in itertools.product([8,10],[False,True],[0,1],['16x16','16x8','8x16','8x8'],range(3),[False,True],range(3)):
            config=configuration(depth,weighted=True,bipred=1);frames=[]
            for index,pts in enumerate([0,4,2]):
                ns=[slice(a,index,depth,identity,selected,mode,filter,init) for a in ([3,1,2,0] if aso else range(4))];frames.append((pts,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            name=f'avc-paff-cabac-weight-{depth}bit-'+('identity' if identity else 'weighted')+f'-ref{selected}-{mode}-init{init}-filter{filter}'+('-aso' if aso else '')
            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames));subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels));data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-paff-cabac-weight-generated.json').write_text(json.dumps(dict(generator='owned PAFF CABAC weighted multireference partitions',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
