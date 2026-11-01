#!/usr/bin/env python3
"""Offline reserved-OBU acceptance qualification from authored AV1 controls."""
import hashlib,json,struct
from pathlib import Path
from generate_av1_show_existing_samples import leb,obu
from generate_audio_resample_window_fixture import ebml
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'av1-reserved-obus'
def pieces(data):
    at=0;out=[]
    while at<len(data):
        start=at;h=data[at];at+=1
        if h&4:at+=1
        assert h&2
        size=0;shift=0
        while True:
            b=data[at];at+=1;size|=(b&127)<<shift;shift+=7
            if b<128:break
        payload=data[at:at+size];at+=size
        assert at<=len(data);out.append((h>>3&15,data[start:at],payload))
    return out

def codec_private(data):
    found=[]
    def vint(at,keep_marker):
        first=data[at];width=1
        while width<=8 and not first&(1<<(8-width)):width+=1
        assert width<=8 and at+width<=len(data)
        value=int.from_bytes(data[at:at+width],'big')
        return (value if keep_marker else value&((1<<(7*width))-1)),at+width
    def walk(start,end):
        at=start
        while at<end:
            kind,at=vint(at,True);size,at=vint(at,False);stop=at+size
            assert stop<=end
            if kind in [0x18538067,0x1654ae6b,0xae]:walk(at,stop)
            elif kind==0x63a2:found.append(data[at:stop])
            at=stop
        assert at==end
    walk(0,len(data));assert len(found)==1
    return found[0]

def movie(packets,private):
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_AV1')+ebml('63a2',private)
    track+=ebml('23e383',(20000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x20')+ebml('ba',b'\x20'))
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets):cluster+=ebml('a3',b'\x81'+(20*i).to_bytes(2,'big')+bytes([(128 if i==0 else 0)|(8 if i==0 else 0)])+p)
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))

def main():
    OUT.mkdir(exist_ok=True)
    parent=json.loads((FIX/'av1-separate-frame-generated.json').read_text())['fixtures'][0]
    source=(FIX/parent['file']).read_bytes();parts=pieces(source);packets=[];current=[];has_frame=False
    for kind,wire,payload in parts:
        if kind==3 and has_frame:packets.append(current);current=[];has_frame=False
        current.append((kind,wire,payload));has_frame|=kind==3
        if kind==1:current.append((5,obu(5,b'\1'+struct.pack('>HH',1000,200)+b'\x80'),b''))
    packets.append(current);assert len(packets)==3
    sequence=next(w for k,w,p in parts if k==1)
    private=codec_private((FIX/parent['webm']).read_bytes());assert private[4:]==sequence
    control=[b''.join(w for k,w,p in row)for row in packets]
    (OUT/'control.obu').write_bytes(b''.join(control));(OUT/'control.webm').write_bytes(movie(control,private))
    (OUT/'display.yuv').write_bytes((FIX/parent['reference']).read_bytes())
    cases=[]
    for kind in [0,9,10,11,12,13,14,15]:
        for extended in [False,True]:
            payload=b'\0'*9 if kind==15 else b'\x80\x0a\xff\0\x32\x80\xca\xfe\x12'
            unknown=bytes([(kind<<3)|2|(4 if extended else 0)])+(b'\0'if extended else b'')+leb(len(payload))+payload
            changed=[b''.join(unknown+w for k,w,p in row)+unknown for row in packets]
            name=f'type{kind}-extension{int(extended)}';raw=b''.join(changed)
            (OUT/(name+'.obu')).write_bytes(raw);(OUT/(name+'.webm')).write_bytes(movie(changed,private))
            assert sum(k==kind for k,w,p in pieces(raw))==sum(len(p)+1 for p in packets)
            cases.append(dict(name=name,kind=kind,extended=extended,insertions=sum(len(p)+1 for p in packets),packet_bytes=[len(p)for p in changed],sha256=hashlib.sha256(raw).hexdigest()))
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,control_packet_bytes=[len(p)for p in control],source=parent['file'],source_sha256=hashlib.sha256(source).hexdigest(),reference=parent['reference'],scope='Sixteen authored three-frame programmes; reserved types0/9..14 and padding15, with/without extension header. Inserted before every known OBU and after each packet; includes pending separate-frame state, hidden reference, inter frames and HDR CLL. Own synthetic parent and saved independent pixels; no private/ISO media, process, FFmpeg or network. Containers preserve all inserted OBUs.'),indent=2)+'\n')
    print('Generated',len(cases),'programmes')
if __name__=='__main__':main()
