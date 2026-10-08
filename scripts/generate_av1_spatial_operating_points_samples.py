#!/usr/bin/env python3
"""Owned spatial SVC fixtures; external reference tools run only during generation."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ['encoder','oracle','second-oracle']: p.add_argument('--'+arg,type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    coded=root/'av1-spatial-operating-points.obu'
    subprocess.run([str(a.encoder),str(coded)],check=True)
    data=coded.read_bytes();layers=[];at=0;sequence=None
    while at<len(data):
        header=data[at];at+=1;kind=(header>>3)&15;sid=0
        if header&4:sid=(data[at]>>3)&3;at+=1
        length=0;shift=0
        while True:
            byte=data[at];at+=1;length|=(byte&127)<<shift;shift+=7
            if not byte&128:break
        payload=data[at:at+length];at+=length
        if kind==1:sequence=payload
        if kind==6:layers.append(sid)
    assert layers==[0,1]*4,layers
    bits=''.join(f'{v:08b}' for v in sequence);assert bits[3:7]=='0000'
    count=int(bits[7:12],2)+1;offset=12;idcs=[]
    for _ in range(count):
        idcs.append(int(bits[offset:offset+12],2));offset+=12;level=int(bits[offset:offset+5],2);offset+=5+int(level>7)
    assert idcs==[0x301,0x101],idcs
    records=[]
    for point,idc in enumerate(idcs):
        expected=root/f'av1-spatial-operating-points-op{point}.yuv'
        subprocess.run([str(a.oracle),f'--oppoint={point}','--all-layers','--i420','--rawvideo',f'--output={expected}',str(coded)],check=True)
        with tempfile.TemporaryDirectory(prefix='fvid-svc-reference-') as tmp:
            cross=Path(tmp)/'cross.yuv'
            subprocess.run([str(a.second_oracle),'--oppoint',str(point),'--alllayers','1','--demuxer','section5','--muxer','yuv','-i',str(coded),'-o',str(cross)],check=True)
            assert cross.read_bytes()==expected.read_bytes(),f'point {point}: independent oracle mismatch'
        indices=[i for i,s in enumerate(layers) if idc&(1<<(s+8))]
        assert expected.stat().st_size==sum((32*24 if layers[i]==0 else 64*48)*3//2 for i in indices)
        records.append(dict(point=point,idc=idc,indices=indices,sizes=[[32,24] if layers[i]==0 else [64,48] for i in indices],reference=expected.name,reference_sha256=hashlib.sha256(expected.read_bytes()).hexdigest()))
    # WebM temporal-unit assembly is qualified separately from raw SVC decoding.
    (root/'av1-spatial-operating-points-generated.json').write_text(json.dumps(dict(file=coded.name,sha256=hashlib.sha256(data).hexdigest(),spatial_ids=layers,size=[64,48],oracles=['libaom','dav1d'],points=records),indent=2)+'\n')
if __name__=='__main__':main()
