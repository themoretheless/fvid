#!/usr/bin/env python3
"""Owned separate-field frame_num gap streams with retained real-reference prediction."""
import argparse,hashlib,itertools,json,subprocess,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration,Writer
from generate_avc_field_reference_samples import intra
from generate_avc_field_cabac_samples import configuration as cabac_config, field
from generate_avc_mbaff_direct_samples import CabacWriter
from avc_fixture_mp4 import mux,annexb

def prediction(bottom,address,number,poc,skip,deblock,init=None,poc_type=0,poc_delta=0):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(number,4);b.u(1);b.u(int(bottom))
    if poc_type==0:b.u(poc,4)
    elif poc_type==1:b.se(poc_delta)
    b.u(0);b.u(1);b.ue(0);b.ue(3 if number in [0,2] else 1);b.ue(3);b.u(0)
    if init is not None:b.ue(init)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if init is not None:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(init,50);c.decision(11,int(skip))
        if not skip:
            for ctx in [14,15,16]:c.decision(ctx,0)
            c.mvd(0,0);c.mvd(1,0)
            for ctx in [73,74,75,76,77]:c.decision(ctx,0)
        b.bits.extend(c.finish());return b.nal(0x41,trailing=False)
    if skip:b.ue(1)
    else:b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(0)
    return b.nal(0x41)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);parser.add_argument('--wrap',action='store_true');parser.add_argument('--long-term',action='store_true');parser.add_argument('--cabac',action='store_true');parser.add_argument('--poc-type',type=int,choices=[0,1,2],default=0);args=parser.parse_args();assert not args.long_term or args.wrap
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-field-gap-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,skip,deblock,aso,init in itertools.product([8,10],[False,True],[False,True],[0,1,2],[False,True],range(3) if args.cabac else [None]):
            frames=[];order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1]
            for stage,number in enumerate([0,14,0] if args.wrap else [0,2,3]):
                for i,bottom in enumerate(order):
                    poc=stage*2+i if args.wrap else number*2+i
                    if stage==0 or (args.wrap and stage==1):
                        ns=[field(bottom,poc,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=stage==1,frame_num=number,long_term=args.long_term and stage==0,poc_type=args.poc_type,poc_delta=i if args.poc_type==1 else 0) for a in addresses] if args.cabac else [intra(bottom,depth,number,stage==0 and i==0,poc,long_term=args.long_term and stage==0,poc_type=args.poc_type,poc_delta=i if args.poc_type==1 else 0)]
                    else:ns=[prediction(bottom,a,number,poc,skip,deblock,init,args.poc_type,i if args.poc_type==1 else 0) for a in addresses]
                    frames.append((poc,stage==0 and i==0,b''.join(len(n).to_bytes(4,'big')+n for n in ns)))
            config=(cabac_config if args.cabac else configuration)(depth,gaps=True,max_refs=4 if args.long_term else 3,poc_type=args.poc_type);family='avc-field-gap-wrap-long-' if args.long_term else 'avc-field-gap-wrap-' if args.wrap else 'avc-field-gap-';family=family+'cabac-' if args.cabac else family;family=family+f'poc{args.poc_type}-' if args.poc_type else family;name=f'{family}{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-skip' if skip else '-coded')+f'-filter{deblock}'+(f'-init{init}' if args.cabac else '')+('-aso' if aso else '')
            coded=d/'sample.264';oracle=d/'sample.yuv';coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
            pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1)
            data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
            if not args.poc_type and not args.cabac and not args.wrap and depth==8 and not reverse and skip and deblock==1 and not aso:
                (root/'avc-field-gap-forbidden.mp4').write_bytes(mux(configuration(depth,gaps=False),frames,32,32,50))
    refusal=[]
    if args.poc_type==1 and not args.cabac and not args.wrap:
        config=configuration(8,gaps=True,poc_type=1,poc_bottom_offset=1)
        frames=[]
        for i,bottom in enumerate([True,False]):
            n=intra(bottom,8,0,i==0,i,poc_type=1,poc_delta=0);frames.append((i,i==0,len(n).to_bytes(4,'big')+n))
        data=mux(config,frames,32,32,50);name='avc-field-poc1-invalid-idr-bottom.mp4';(root/name).write_bytes(data)
        refusal=[dict(file=name,sha256=hashlib.sha256(data).hexdigest(),reason='IDR bottom field POC is not zero')]
    manifest=dict(generator='owned field gap prediction',fixtures=records)
    if refusal:manifest['refusals']=refusal
    (root/(family+'generated.json')).write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':main()
