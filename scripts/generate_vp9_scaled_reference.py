#!/usr/bin/env python3
"""Owned VP9 resize headers on frozen public synthetic entropy; offline."""
import json
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
ROOT=Path(__file__).resolve().parents[1]
DEST=ROOT/'tests/fixtures/playback-errors'
def packed(bits):
    bits+='0'*(-len(bits)%8)
    return bytes(int(bits[i:i+8],2) for i in range(0,len(bits),8))
def webm(packets):
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')
    track+=ebml('23e383',(333333333).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x20')+ebml('ba',b'\x20'))
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets):cluster+=ebml('a3',b'\x81'+(i*333).to_bytes(2,'big')+bytes([128 if i==0 else 0])+p)
    info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
def main():
    data=(ROOT/'tests/fixtures/vp9/header.ivf').read_bytes();at=32;packets=[]
    while at<len(data):
        n=int.from_bytes(data[at:at+4],'little');at+=12;packets.append(data[at:at+n]);at+=n
    assert len(packets)==3
    cases=[]
    for label,size in [('horizontal',(31,32)),('vertical',(32,31)),('both',(31,31)),('bad-ratio',(15,32))]:
        outputs=[packets[0]]
        for index,p in enumerate(packets[1:]):
            value=''.join(f'{b:08b}' for b in p[:10]);assert value[30]=='1' and int(value[-16:],2)==3
            w,h=size if index==0 else (32,32)
            header=value[:30]+'000'+f'{w-1:016b}{h-1:016b}'+value[31:]
            outputs.append(packed(header)+p[10:])
        name=f'vp9-scaled-reference-{label}-synthetic.webm'
        (DEST/name).write_bytes(webm(outputs))
        cases.append(dict(file=name,expectation='invalid' if label=='bad-ratio' else 'acceptance',sizes=[[32,32],list(size),[32,32]],failure_picture=1 if label=='bad-ratio' else None))
    (DEST/'vp9-scaled-reference.json').write_text(json.dumps(dict(cases=cases,provenance='Frozen public testsrc2 seed documented in tests/fixtures/vp9/README.md. Only explicit frame sizes and WebM tables authored; original entropy retained. No private media, external codec invocation, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
