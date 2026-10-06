#!/usr/bin/env python3
"""Owned CAVLC B fields mixing direct, explicit subpartitions and I_PCM."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import configuration
from generate_avc_field_reference_samples import intra
from generate_avc_field_b_direct_samples import p_field
from generate_avc_mbaff_direct_samples import Writer, pcm_samples
from avc_fixture_mp4 import mux, annexb


def prediction(b, codes, vector):
    # All reference groups precede MVDs; direct groups have neither syntax.
    def mode(code):
        return None if code==0 else code if code<=3 else (code-4)//2+1 if code<=9 else code-9
    def count(code):
        return 0 if code==0 else 1 if code<=3 else 2 if code<=9 else 4
    for list_index in [0,1]:
        for group,code in enumerate(codes):
            used=mode(code) in ([1,3] if list_index==0 else [2,3])
            if used:b.ue(1 if len(codes)==1 else (group+list_index)%4)
    for list_index in [0,1]:
        for code in codes:
            used=mode(code) in ([1,3] if list_index==0 else [2,3])
            if used:
                for part in range(count(code)):
                    b.se((3 if list_index==0 else -2)*vector*(1 if part%2==0 else -1))
                    b.se((-2 if list_index==0 else 3)*vector)


def b_field(bottom,index,depth,spatial,kind,code,position,vector,deblock):
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(index,4)
    b.u(int(spatial));b.u(1);b.ue(3);b.ue(3);b.u(0);b.u(0);b.se(24);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    for address in range(2):
        b.ue(0)
        if kind=='pcm' and address==position:
            b.ue(48);b.align();b.bits.extend(pcm_samples(depth,int(bottom)+6,address));continue
        if kind=='adjacent' and address==position:
            b.ue(code);prediction(b,[code],vector)
        elif kind=='sub':
            b.ue(22);codes=[0 if group==position else code for group in range(4)]
            for c in codes:b.ue(c)
            prediction(b,codes,vector)
        else:b.ue(0)
        b.ue(0)
    return b.nal(0x01)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    variants=[('adjacent',code,pos,v) for code in [1,2,3] for pos in [0,1] for v in [0,1]]+ [('pcm',0,pos,1) for pos in [0,1]]+ [('sub',code,pos,1) for code in range(1,13) for pos in range(4)]
    with tempfile.TemporaryDirectory(prefix='fvid-mixed-b-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth in [8,10]:
            for reverse in [False,True]:
                for spatial in [False,True]:
                    for inference in [False,True]:
                        for deblock in [0,1,2]:
                            for kind,code,pos,vector in variants:
                                order=[True,False] if reverse else [False,True];frames=[]
                                for number in [0,1]:
                                    for i,bottom in enumerate(order):
                                        index=number*4+i;n=intra(bottom,depth,number,index==0,index)
                                        frames.append((index,index==0,len(n).to_bytes(4,'big')+n))
                                for i,bottom in enumerate(order):
                                    nals=[p_field(bottom,8+i,a,deblock) for a in [0,1]]
                                    frames.append((8+i,False,b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
                                for i,bottom in enumerate(order):
                                    n=b_field(bottom,6+i,depth,spatial,kind,code,pos,vector,deblock)
                                    frames.append((6+i,False,len(n).to_bytes(4,'big')+n))
                                config=configuration(depth,direct8=inference)
                                name=f'avc-field-b-mixed-{depth}bit-'+('bottom-first' if reverse else 'top-first')+('-spatial' if spatial else '-temporal')+f'-{kind}-type{code}-pos{pos}-v{vector}-filter{deblock}-infer{int(inference)}'
                                coded=d/(name+'.264');oracle=d/(name+'.yuv');coded.write_bytes(annexb(config,frames))
                                subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=d,check=True,stdout=subprocess.DEVNULL)
                                pixels=oracle.read_bytes();assert len(pixels)==6144*(2 if depth==10 else 1),(name,len(pixels))
                                data=mux(config,frames,32,32,50);(root/(name+'.mp4')).write_bytes(data);(root/(name+'.yuv')).write_bytes(pixels)
                                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (root/'avc-field-b-mixed-generated.json').write_text(json.dumps(dict(generator='owned CAVLC mixed B-field writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
