#!/usr/bin/env python3
"""Owned explicit L0/L1/Bi B fields, separate CAVLC/CABAC syntax and JM oracle."""
import argparse, tempfile, subprocess, hashlib, json
from pathlib import Path
from generate_avc_field_cabac_samples import configuration as cabac_configuration, field
from generate_avc_field_reference_samples import configuration as cavlc_configuration, intra
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def prediction(bottom, index, address, mode, deblock, init):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(1);b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0)
    if init is not None:b.ue(init)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    code={'l0':1,'l1':2,'bi':3}[mode]
    lists=[0] if mode=='l0' else [1] if mode=='l1' else [0,1]
    if init is None:
        b.ue(0);b.ue(code)
        for list_index in lists:b.ue((address+list_index)%4)
        for list_index in lists:b.se(1 if list_index==0 else -1);b.se(-1 if list_index==0 else 1)
        b.ue(0)
        return b.nal(0x01)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(24,0)
    for i,symbol in enumerate({1:'100',2:'101',3:'110000'}[code]):
        c.decision(27 if i==0 else 30 if i==1 else 31 if i==2 and mode=='bi' else 32,int(symbol))
    for list_index in lists:
        reference=(address+list_index)%4
        for symbol in range(reference+1):c.decision(54 if symbol==0 else 58 if symbol==1 else 59,int(symbol<reference))
    for list_index in lists:c.mvd(0,1 if list_index==0 else -1,0);c.mvd(1,-1 if list_index==0 else 1,0)
    for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);p.add_argument('--future',action='store_true',help='Decode future reference fields before B fields; emit reordered PTS');p.add_argument('--implicit',action='store_true',help='Implicit B weights with future-reference POC; separate fixtures');args=p.parse_args();args.future=args.future or args.implicit
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-explicit-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for mode in ['l0','l1','bi']:
                    for init in [None,0,1,2]:
                        for deblock in [0,1,2]:
                            for aso in [False,True]:
                                order=[True,False] if reverse else [False,True];frames=[]
                                for number in [0,1]:
                                    for i,bottom in enumerate(order):
                                        index=(4 if args.future and number==1 else 2*number)+i
                                        if init is None:
                                            nals=[intra(bottom,depth,number,index==0,index)]
                                        else:
                                            nals=[field(bottom,index,address,'positive' if (address==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for address in ([1,0] if aso else [0,1])]
                                        frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                for i,bottom in enumerate(order):
                                    nals=[prediction(bottom,(2 if args.future else 4)+i,address,mode,deblock,init) for address in ([1,0] if aso else [0,1])]
                                    frames.append(((2 if args.future else 4)+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                config=cavlc_configuration(depth,bipred=2 if args.implicit else 0) if init is None else cabac_configuration(depth,bipred=2 if args.implicit else 0)
                                entropy='cavlc' if init is None else f'cabac-init{init}'
                                prefix='avc-field-b-implicit-' if args.implicit else 'avc-field-b-future-' if args.future else 'avc-field-b-explicit-'
                                name=f'{prefix}{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{mode}-{entropy}-filter{deblock}'+('-aso' if aso else '')
                                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                pixels=oracle.read_bytes();assert len(pixels)==4608*(2 if depth==10 else 1),(name,len(pixels))
                                data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
                                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/('avc-field-b-implicit-generated.json' if args.implicit else 'avc-field-b-future-generated.json' if args.future else 'avc-field-b-explicit-generated.json')).write_text(json.dumps(dict(generator='owned explicit B-field writer',fixtures=records),indent=2)+'\n')

if __name__=='__main__':main()
