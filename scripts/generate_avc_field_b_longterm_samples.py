#!/usr/bin/env python3
"""Owned separate B direct fields isolating long-term source and co-located flags."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration as cavlc_config
from generate_avc_field_reference_samples import intra
from generate_avc_field_cabac_samples import configuration as cabac_config, field
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def lists(b,markers,current,target):
    b.u(1);pred=current
    for pos,long_term in enumerate(markers):
        if long_term:b.ue(2);b.ue(1-pos)
        else:b.ue(0);b.ue(pred-(target-pos)-1);pred=target-pos
    b.ue(3)


def prediction(bottom,index,spatial,case,long_term,skip,deblock,init,is_b,mixed=False,first_bottom=False):
    source=case=='source' and long_term;colocated=case=='colocated' and long_term
    current=not mixed or bottom==first_bottom
    opposite=not mixed or bottom!=first_bottom
    b=Writer();b.ue(0);b.ue(1 if is_b else 0);b.ue(0);b.u(3 if is_b else 2,4);b.u(1);b.u(int(bottom));b.u(index,4)
    if is_b:b.u(int(spatial))
    b.u(1);b.ue(1)
    if is_b:b.ue(1)
    lists(b,(source and current,source and opposite),7 if is_b else 5,1)
    if is_b:lists(b,(colocated and current,colocated and opposite),7,5)
    else:
        if colocated and current:b.u(1);b.ue(4);b.ue(1);b.ue(6);b.ue(0);b.ue(0)
        else:b.u(0)
    if init is not None:b.ue(init)
    b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    vector=(4,-2) if case=='source' else (1,1)
    if init is None:
        for address in range(2):
            if is_b and address==1 and skip:b.ue(1);continue
            b.ue(0);b.ue(1 if is_b and address==0 else 0); # B L0 then direct; P16 in both.
            if not is_b or address==0:
                b.u(1) # truncated ref_idx_l0=0 for active count2
                mv=(3,-2) if is_b else vector if address==0 else (0,0)
                b.se(mv[0]);b.se(mv[1])
            b.ue(0)
        return b.nal(0x01 if is_b else 0x41)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50)
    for address in range(2):
        if is_b:
            c.decision(24+address,int(address==1 and skip))
            if address==0:
                for ctx,symbol in [(27,1),(30,0),(32,0)]:c.decision(ctx,symbol)
                c.decision(54,0);c.mvd(0,3,0);c.mvd(1,-2,0)
            elif not skip:c.decision(28,0)
        else:
            c.decision(11+address,0);c.decision(14,0);c.decision(15,0);c.decision(16,0);c.decision(54,0)
            c.mvd(0,vector[0] if address==0 else 0,0 if address==0 else abs(vector[0]));c.mvd(1,vector[1] if address==0 else 0,0 if address==0 else abs(vector[1]))
        if not (is_b and address==1 and skip):
            for ctx in ([73,74,75,76] if address==0 else [74,74,76,76]):c.decision(ctx,0)
            c.decision(77,0)
        if address==0:c.range-=2;c.renormalize()
    b.bits.extend(c.finish());return b.nal(0x01 if is_b else 0x41,trailing=False)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);parser.add_argument('--mixed-parity',action='store_true');args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-longterm-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for init in [None,0,1,2]:
                    for spatial in [False,True]:
                        for case in ['source','colocated']:
                            for long_term in [False,True]:
                                for skip in [False,True]:
                                    for deblock in [0,1,2]:
                                        order=[True,False] if reverse else [False,True];frames=[]
                                        for number in [0,1]:
                                            for i,bottom in enumerate(order):
                                                index=number*4+i;mark=number==0 and case=='source' and long_term and (not args.mixed_parity or i==0)
                                                nals=[intra(bottom,depth,number,index==0,index,long_term=mark)] if init is None else [field(bottom,index,a,'positive' if (a==0)^bottom else 'negative',deblock,biased=number==1,frame_num=number,long_term=mark) for a in [0,1]]
                                                frames.append((index,index==0,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                        for is_b in [False,True]:
                                            for i,bottom in enumerate(order):
                                                index=(6 if is_b else 8)+i;n=prediction(bottom,index,spatial,case,long_term,skip,deblock,init,is_b,args.mixed_parity,order[0]);frames.append((index,False,len(n).to_bytes(4,'big')+n))
                                        config=(cavlc_config if init is None else cabac_config)(depth,max_refs=4 if args.mixed_parity else 3);entropy='cavlc' if init is None else f'cabac-init{init}'
                                        prefix='avc-field-b-longterm-mixed-' if args.mixed_parity else 'avc-field-b-longterm-'
                                        name=f'{prefix}{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+f'-{case}-'+('long' if long_term else 'short')+('-skip' if skip else '-coded')+f'-{entropy}-filter{deblock}'
                                        coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                        subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                        pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                        data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels);records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/('avc-field-b-longterm-mixed-generated.json' if args.mixed_parity else 'avc-field-b-longterm-generated.json')).write_text(json.dumps(dict(generator='owned long-term source and co-located field direct controls',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
