#!/usr/bin/env python3
"""Owned CABAC B-direct field 8x8 residual, signs and inter scaling controls."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_cabac_p_transform8_samples import configuration
from generate_avc_field_cabac_samples import field
from generate_avc_field_cabac_p_multiref_samples import prediction as p_field
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def b_field(bottom,index,address,spatial,coded,negative,deblock,init,qp=50):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0);b.ue(init);b.se(qp-26);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,max(0,qp));c.decision(24,0);c.decision(27,0)
    if coded:
        for _ in range(4):c.decision(73,1)
        c.decision(77,0);c.decision(399,1);c.decision(60,0)
        for block in range(4):
            c.decision(436,0);c.decision(437,1);c.decision(452,1);c.decision(427,0);c.bypass((block%2)^negative)
    else:
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-transform8-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for spatial in [False,True]:
                    for init in range(3):
                        for deblock in [0,1,2]:
                            for aso in [False,True]:
                                for scaled in [False,True]:
                                    for coded,negative in [(False,0),(True,0),(True,1)]:
                                        order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
                                        for number in [0,1]:
                                            for i,bottom in enumerate(order):
                                                index=number*4+i;nals=[field(bottom,index,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for a in addresses]
                                                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        for i,bottom in enumerate(order):
                                            nals=[p_field(bottom,8+i,a,'4x4',deblock,init,True,True) for a in addresses];frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        for i,bottom in enumerate(order):
                                            nals=[b_field(bottom,6+i,a,spatial,coded,negative,deblock,init) for a in addresses];frames.append((6+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        config=configuration(depth,scaled)
                                        name=f'avc-field-b-transform8-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+('-ac' if coded else '-none')+f'-sign{negative}-init{init}-filter{deblock}-scale'+('24' if scaled else '16')+('-aso' if aso else '')
                                        coded_file=d/(name+'.264');oracle=d/(name+'.yuv');coded_file.write_bytes(annexb(config,frames))
                                        subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded_file}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                        pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                        data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
                                        records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-field-b-transform8-generated.json').write_text(json.dumps(dict(generator='owned CABAC B direct signed 8x8 field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
