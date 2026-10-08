#!/usr/bin/env python3
"""Owned temporal SVC fixtures; external reference tools run only during generation."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ['encoder','oracle','second-oracle']: p.add_argument('--'+arg,type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    coded=root/'av1-temporal-operating-points.obu'
    subprocess.run([str(a.encoder),str(coded)],check=True)
    data=coded.read_bytes();tids=[];at=0;sequence=None
    while at<len(data):
        header=data[at];at+=1;kind=(header>>3)&15;tid=0
        if header&4:tid=data[at]>>5;at+=1
        length=0;shift=0
        while True:
            byte=data[at];at+=1;length|=(byte&127)<<shift;shift+=7
            if not byte&128:break
        payload=data[at:at+length];at+=length
        if kind==1:sequence=payload
        if kind==6:tids.append(tid)
    assert tids==[0,2,1,2,0,2,1,2],tids
    bits=''.join(f'{v:08b}' for v in sequence);assert bits[3:7]=='0000'
    count=int(bits[7:12],2)+1;offset=12;idcs=[]
    for _ in range(count):
        idcs.append(int(bits[offset:offset+12],2));offset+=12;level=int(bits[offset:offset+5],2);offset+=5+int(level>7)
    assert idcs==[0x107,0x103,0x101],idcs
    records=[]
    for point,idc in enumerate(idcs):
        expected=root/f'av1-temporal-operating-points-op{point}.yuv'
        subprocess.run([str(a.oracle),f'--oppoint={point}','--i420','--rawvideo',f'--output={expected}',str(coded)],check=True)
        with tempfile.TemporaryDirectory(prefix='fvid-svc-reference-') as tmp:
            cross=Path(tmp)/'cross.yuv'
            subprocess.run([str(a.second_oracle),'--oppoint',str(point),'--demuxer','section5','--muxer','yuv','-i',str(coded),'-o',str(cross)],check=True)
            assert cross.read_bytes()==expected.read_bytes(),f'point {point}: independent oracle mismatch'
        indices=[i for i,t in enumerate(tids) if idc&(1<<t)]
        assert expected.stat().st_size==len(indices)*64*48*3//2
        records.append(dict(point=point,idc=idc,indices=indices,reference=expected.name,reference_sha256=hashlib.sha256(expected.read_bytes()).hexdigest()))
    wrapped=root/'av1-temporal-operating-points.webm';wrapped.write_bytes(webm(data,(64,48)))
    (root/'av1-temporal-operating-points-generated.json').write_text(json.dumps(dict(file=coded.name,sha256=hashlib.sha256(data).hexdigest(),webm=wrapped.name,webm_sha256=hashlib.sha256(wrapped.read_bytes()).hexdigest(),temporal_ids=tids,size=[64,48],oracles=['libaom','dav1d'],points=records),indent=2)+'\n')
if __name__=='__main__':main()
