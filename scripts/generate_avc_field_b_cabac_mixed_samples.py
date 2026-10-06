#!/usr/bin/env python3
"""Owned single-slice CABAC B fields mixing direct with explicit or intra blocks."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration, field
from generate_avc_field_cabac_p_multiref_samples import prediction as p_field
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def mb_type(c,code,increment):
    bits={0:'0',1:'100',2:'101',3:'110000',23:'111101'}[code]
    prior=0
    for i,symbol in enumerate(bits):
        context=27+increment if i==0 else 30 if i==1 else (31 if prior&1 else 32) if i==2 else 32
        c.decision(context,int(symbol));prior=prior*2+int(symbol)


def b_field(bottom,index,spatial,mode,position,skip,vector,deblock,init):
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0);b.ue(init);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50)
    for address in range(2):
        selected=address==position;is_skip=not selected and skip
        previous_skip=address==1 and position==1 and skip
        c.decision(24+int(address==1 and not previous_skip),int(is_skip))
        if selected:
            intra=mode.startswith('i')
            mb_type(c,23 if intra else {'l0':1,'l1':2,'bi':3}[mode],0)
            if intra:
                c.decision(32,int(mode!='i4-zero'))
                if mode!='i4-zero':
                    c.range-=2;c.renormalize();c.decision(33,0);c.decision(34,0);c.decision(35,1);c.decision(35,0)
                else:
                    for _ in range(16):c.decision(68,1)
                c.decision(64,0)
                if mode!='i4-zero':
                    c.decision(60,0);c.decision(88 if address==0 else 87,1);c.decision(277,0);c.decision(278,1);c.decision(339,1);c.decision(228,0);c.bypass(int(mode=='i16-negative'))
            else:
                lists=[0] if mode=='l0' else [1] if mode=='l1' else [0,1]
                for _ in lists:c.decision(54,1);c.decision(58,0)
                for list_index in lists:c.mvd(0,(3 if list_index==0 else -2)*vector,0);c.mvd(1,(-2 if list_index==0 else 3)*vector,0)
            if not intra or mode=='i4-zero':
                for ctx in ([73,74,75,76] if address==0 else [74,74,76,76]):c.decision(ctx,0)
                c.decision(77,0)
        elif not skip:
            mb_type(c,0,int(address==1))
            for ctx in ([73,74,75,76] if address==0 else [74,74,76,76]):c.decision(ctx,0)
            c.decision(77,0)
        if address==0:c.range-=2;c.renormalize()
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    variants=[(m,v) for m in ['l0','l1','bi'] for v in [0,1]]+[(m,1) for m in ['i4-zero','i16-positive','i16-negative']]
    with tempfile.TemporaryDirectory(prefix='fvid-cabac-mixed-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for spatial in [False,True]:
                    for inference in [False,True]:
                        for init in range(3):
                            for deblock in [0,1,2]:
                                for position in [0,1]:
                                    for skip in [False,True]:
                                        for mode,vector in variants:
                                            order=[True,False] if reverse else [False,True];frames=[]
                                            for number in [0,1]:
                                                for i,bottom in enumerate(order):
                                                    index=number*4+i;nals=[field(bottom,index,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number) for a in [0,1]];frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                            for i,bottom in enumerate(order):
                                                nals=[p_field(bottom,8+i,a,'4x4',deblock,init,True,True) for a in [0,1]];frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                            for i,bottom in enumerate(order):
                                                n=b_field(bottom,6+i,spatial,mode,position,skip,vector,deblock,init);frames.append((6+i,False,len(n).to_bytes(4,'big')+n))
                                            config=configuration(depth,direct8=inference)
                                            name=f'avc-field-b-cabac-mixed-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+f'-{mode}-pos{position}-'+('skip' if skip else 'coded')+f'-v{vector}-init{init}-filter{deblock}-infer{int(inference)}'
                                            coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                            pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                            data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-field-b-cabac-mixed-generated.json').write_text(json.dumps(dict(generator='owned CABAC mixed direct explicit and intra B fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
