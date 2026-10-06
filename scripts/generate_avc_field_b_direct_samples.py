#!/usr/bin/env python3
"""Owned separate B direct/skip fields with inter-coded co-located references."""
import argparse, tempfile, subprocess, hashlib, json
from pathlib import Path
from generate_avc_field_pcm_samples import configuration as cavlc_config
from generate_avc_field_reference_samples import intra
from generate_avc_field_cabac_samples import configuration as cabac_config, field
from generate_avc_field_cabac_p_multiref_samples import prediction as cabac_p
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def p_field(bottom,index,address,deblock):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(2,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(1);b.ue(3);b.u(1)
    for op,value in [(0,3),(0,0),(1,2),(0,0)]:b.ue(op);b.ue(value)
    b.ue(3);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    b.ue(0);b.ue(0);b.ue(address+1);b.se(5 if address==0 else -3);b.se(-3 if address==0 else 5);b.ue(0)
    return b.nal(0x41)


def b_field(bottom,index,address,spatial,skip,deblock,init):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0)
    if init is not None:b.ue(init)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if init is None:
        b.ue(1 if skip else 0)
        if not skip:b.ue(0);b.ue(0)
        return b.nal(0x01)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(24,int(skip))
    if not skip:
        c.decision(27,0)
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,required=True)
    args=parser.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-b-direct-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for spatial in [False,True]:
                    for skip in [False,True]:
                        for init in [None,0,1,2]:
                            for deblock in [0,1,2]:
                                for aso in [False,True]:
                                    for inference in [False,True]:
                                        order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
                                        for number in [0,1]:
                                            for i,bottom in enumerate(order):
                                                index=number*4+i
                                                nals=[intra(bottom,depth,number,index==0,index)] if init is None else [field(bottom,index,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for a in addresses]
                                                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        for i,bottom in enumerate(order):
                                            nals=[p_field(bottom,8+i,a,deblock) if init is None else cabac_p(bottom,8+i,a,'4x4',deblock,init,True,True) for a in addresses]
                                            frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        for i,bottom in enumerate(order):
                                            nals=[b_field(bottom,6+i,a,spatial,skip,deblock,init) for a in addresses]
                                            frames.append((6+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        config=(cavlc_config if init is None else cabac_config)(depth,direct8=inference)
                                        entropy='cavlc' if init is None else f'cabac-init{init}'
                                        name=f'avc-field-b-direct-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+('-skip' if skip else '-coded')+f'-{entropy}-filter{deblock}-infer{int(inference)}'+('-aso' if aso else '')
                                        coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                        subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                        pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                        data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
                                        records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-field-b-direct-generated.json').write_text(json.dumps(dict(generator='owned separate-field direct writer; inter-coded colocated fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
