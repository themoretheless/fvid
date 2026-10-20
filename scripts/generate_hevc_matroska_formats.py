#!/usr/bin/env python3
"""Matroska wrappers and independent display oracles for authored HEVC PCM fixtures.
No encoding, external codec, production decoder, private media or network.
"""
import hashlib,json,struct
from pathlib import Path
from generate_ffv1_gray_fixtures import element,uint
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def boxes(data):
    at=0
    while at<len(data):
        size=int.from_bytes(data[at:at+4],'big');tag=data[at+4:at+8]
        assert size>=8 and at+size<=len(data)
        yield tag,data[at+8:at+size];at+=size
    assert at==len(data)
def child(data,tag):return next(v for k,v in boxes(data) if k==tag)
def unpack(path):
    data=path.read_bytes();moov=child(data,b'moov');trak=child(moov,b'trak');mdia=child(trak,b'mdia');stbl=child(child(mdia,b'minf'),b'stbl')
    stsd=child(stbl,b'stsd');entry=next(boxes(stsd[8:]))[1];private=child(entry[78:],b'hvcC')
    width,height=struct.unpack('>HH',entry[24:28]);rate=int.from_bytes(child(mdia,b'mdhd')[12:16],'big')
    stsz=child(stbl,b'stsz');fixed,count=struct.unpack('>II',stsz[4:12]);sizes=[fixed]*count if fixed else list(struct.unpack('>'+('I'*count),stsz[12:]))
    stco=child(stbl,b'stco');assert int.from_bytes(stco[4:8],'big')==1;offset=int.from_bytes(stco[8:12],'big');packets=[]
    for size in sizes:packets.append(data[offset:offset+size]);offset+=size
    return private,packets,width,height,rate

def movie(private,packets,width,height,rate,rotation,crop):
    video=uint(0xb0,width)+uint(0xba,height)
    video+=b''.join(uint(tag,n) for tag,n in zip([0x54cc,0x54bb,0x54dd,0x54aa],crop))
    video+=element(0x7670,uint(0x7671,0)+element(0x7675,struct.pack('>d',90. if rotation==270 else -float(rotation))))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,1000000000//rate)+element(0xe0,video)
    blocks=b''.join(element(0xa3,b'\x81'+(i*1000//rate).to_bytes(2,'big')+(b'\x80' if i==0 else b'\0')+p) for i,p in enumerate(packets))
    return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+blocks))
def display(values,width,height,sub,crop,rotation):
    result=[];at=0;left,top,right,bottom=crop
    for sx,sy in [(1,1),sub,sub]:
        w,h=width//sx,height//sy;plane=values[at:at+w*h];at+=w*h
        rows=[plane[y*w+left//sx:y*w+(width-right)//sx] for y in range(top//sy,(height-bottom)//sy)]
        if rotation==180:rows=[r[::-1] for r in rows[::-1]]
        elif rotation==90:rows=[list(r) for r in zip(*rows[::-1])]
        elif rotation==270:rows=[list(r) for r in zip(*rows)][::-1]
        result+=sum(rows,[])
    assert at==len(values)
    return result

def main():
    cases=[]
    sources=[(prefix,suffix,fmt,depth) for prefix,fmt in [('hevc-pcm-444',3),('hevc-pcm-422',2),('hevc-pcm-mono',0),('hevc-pcm',1)] for suffix,depth in [('reference-rext8',8),('full10-rext10',10),('full12-rext12',12)]]
    sources += [('hevc-pcm-444',f'deep-full-rext{depth}',3,depth) for depth in [14,16]]
    for prefix,suffix,fmt,depth in sources:
        source=f'{prefix}-{suffix}.mp4';oracle=source[:-4]+'.yuv';private,packets,width,height,rate=unpack(D/source)
        sub={0:(2,2),1:(2,2),2:(2,1),3:(1,1)}[fmt];unit=1 if depth==8 else 2;raw=(D/oracle).read_bytes()
        source_samples=width*height+(0 if fmt==0 else 2*(width//sub[0])*(height//sub[1]));assert len(raw)==len(packets)*source_samples*unit
        for rotation in [0,90,180,270]:
            for crop in [[0,0,0,0],[2,4,6,8]]:
                name=source[:-4]+f'-matroska-r{rotation}-crop{crop[0]}';data=movie(private,packets,width,height,rate,rotation,crop)
                file=name+'.mkv';(D/file).write_bytes(data);frames=[]
                for i in range(len(packets)):
                    frame=raw[i*source_samples*unit:(i+1)*source_samples*unit]
                    values=list(frame) if unit==1 else [int.from_bytes(frame[j:j+2],'little') for j in range(0,len(frame),2)]
                    if fmt==0:values += [1<<(depth-1)]*(2*(width//2)*(height//2))
                    serialize=lambda v:bytes(v) if unit==1 else b''.join(n.to_bytes(2,'little') for n in v)
                    paths={}
                    for key,content in [('packed',serialize(values)),('display',serialize(display(values,width,height,sub,crop,rotation)))]:
                        target=f'{name}-{i}.{key}';(D/target).write_bytes(content);paths[key]=target
                    frames.append(paths)
                cases.append(dict(file=file,source=source,oracle=oracle,size=[width,height],depth=depth,chroma_format=fmt,subsampling=sub,rotation=rotation,crop=crop,rate=rate,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    (D/'hevc-matroska-formats.json').write_text(json.dumps(dict(cases=cases,provenance='Only already authored HEVC PCM packets/HM synthetic sample oracles are reused. Own MP4 extraction/Matroska wrapping, independent per-plane crop/rotation arrays. No private media, external codec execution or network.'),indent=2)+'\n')
if __name__=='__main__':main()
