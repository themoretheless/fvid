#!/usr/bin/env python3
"""Owned Extended-profile FMO mixed inter/I16 luma AC and chroma residual fixtures."""
import argparse,subprocess,tempfile,hashlib,json
from pathlib import Path
from generate_avc_fmo_samples import configuration
from generate_avc_mbaff_direct_samples import Writer,pcm_samples
from avc_fixture_mp4 import mux,annexb

def slice_nal(kind,group,deblock=1,qp=26,addresses=None,map_type=6,width_mbs=2,mode="all-ac"):
    addresses=[group,group+2] if addresses is None else addresses
    b=Writer();b.ue(addresses[0]);b.ue(2 if kind=='I' else 0 if kind=='P' else 1);b.ue(0)
    b.u({'I':0,'P':1,'B':2}[kind],4)
    if kind=='I':b.ue(0)
    b.u({'I':0,'P':4,'B':2}[kind],4)
    if kind=='B':b.u(0)
    if kind!='I':
        b.u(0);b.u(0)
        if kind=='B':b.u(0)
    if kind=='I':b.u(0);b.u(0)
    elif kind=='P':b.u(0)
    b.se(qp-26);b.ue(deblock)
    if deblock!=1:b.se(6);b.se(6)
    if map_type in [3,4,5]:b.u(2,3)
    if kind=='I':
        for address in addresses:b.ue(25);b.align();b.bits.extend(pcm_samples(8,address//width_mbs,address%width_mbs))
    else:
        b.ue(0);b.ue(0 if kind=='P' else 1);b.se((8 if kind=='P' else 4)+(group*4 if deblock!=1 else 0));b.se(4 if kind=='P' else 0);b.ue(0)
        if len(addresses)>1:
            b.ue(0) # no skip before embedded I16 DC block
            intra_type={'luma-ac':15,'chroma-dc':7,'all-ac':23}[mode]
            b.ue(intra_type+(5 if kind=='P' else 23));b.ue(0);b.se(0)
            b.u(1,2);b.u(0);b.u(1) # nC=0, one positive DC trailing one, no zeros
            if mode in ['luma-ac','all-ac']:
                for i in range(16):b.u(1,2);b.u(i%2);b.u(1) # signed first AC, nC <= 1
            if mode in ['chroma-dc','all-ac']:
                for c in range(2):b.u(1);b.u(c);b.u(1) # 420 DC token (1,1), sign, zero zeros
            if mode=='all-ac':
                for c in range(2):
                    for i in range(4):b.u(1,2);b.u((i+c)%2);b.u(1)
            if len(addresses)>2:b.ue(len(addresses)-2)
    return b.nal(0x65 if kind=='I' else 0x41 if kind=='P' else 0x01)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--jm-decoder',type=Path,required=True);args=parser.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-fmo-mixed-ac-') as tmp:
        directory=Path(tmp);cfg=directory/'decoder.cfg';cfg.write_text('')
        for map_type,direction,reverse,deblock,mode in [(m,d,r,f,mode) for mode in ["luma-ac","chroma-dc","all-ac"] for m in range(7) for d in ([False,True] if m in [3,4,5] else [False]) for r in [False,True] for f in [0,1,2]]:
            config=configuration(map_type,direction,poc_type=0,profile=88,max_refs=2)
            maps={0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]}
            mapping=maps[map_type]
            frames=[]
            for kind,pts in [('I',0),('P',2),('B',1)]:
                nals=[slice_nal(kind,g,deblock=deblock,qp=26 if deblock==1 else 50,addresses=[i for i,v in enumerate(mapping) if v==g],map_type=map_type,mode=mode) for g in ([1,0] if reverse else [0,1])]
                frames.append((pts,kind=='I',b''.join(len(n).to_bytes(4,'big')+n for n in nals)))
            name=f'avc-fmo-mixed-{mode}-type{map_type}-dir{int(direction)}-filter{deblock}'+('-aso' if reverse else '')
            coded=directory/(name+'.264');oracle=directory/(name+'.yuv');coded.write_bytes(annexb(config,frames))
            subprocess.run([str(args.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
            pixels=oracle.read_bytes();assert len(pixels)==4608
            data=mux(config,frames,32,32,25)
            (output/(name+'.mp4')).write_bytes(data);(output/(name+'.yuv')).write_bytes(pixels)
            records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-fmo-mixed-ac-generated.json').write_text(json.dumps(dict(generator='owned Extended-profile FMO mixed motion/I16 signed AC/chroma writer',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
