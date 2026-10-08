#!/usr/bin/env python3
"""Owned spatial SVC container wrappers. No external codecs needed for wrapping."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import atom,word,ebml
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def pieces(data):
    at=0
    while at<len(data):
        begin=at;h=data[at];at+=1;sid=0
        if h&4:sid=(data[at]>>3)&3;at+=1
        n=0;shift=0
        while True:
            b=data[at];at+=1;n|=(b&127)<<shift;shift+=7
            if not b&128:break
        at+=n;yield (h>>3)&15,sid,data[begin:at]
def webm(packets,seq,size=(64,48)):
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_AV1')+ebml('63a2',bytes([0x81,0,0,0])+seq)
    track+=ebml('23e383',(20000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',size[0].to_bytes(max(1,(size[0].bit_length()+7)//8),'big'))+ebml('ba',size[1].to_bytes(max(1,(size[1].bit_length()+7)//8),'big')))
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets):cluster+=ebml('a3',b'\x81'+(i*20).to_bytes(2,'big')+bytes([128 if i==0 else 0])+p)
    info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
def mp4(packets,seq,size=(64,48)):
    count=len(packets)
    ftyp=atom(b'ftyp',b'isom'+word(0)+b'isomav01');mdat=atom(b'mdat',b''.join(packets))
    entry=bytearray(78);entry[6:8]=(1).to_bytes(2,'big');entry[24:28]=struct.pack('>HH',*size);entry[40:42]=(1).to_bytes(2,'big');entry[74:76]=(24).to_bytes(2,'big')
    stsd=atom(b'stsd',word(0,1)+atom(b'av01',entry+atom(b'av1C',bytes([0x81,0,0,0])+seq)))
    stbl=atom(b'stbl',stsd+atom(b'stts',word(0,1,count,1))+atom(b'stsz',word(0,0,count,*map(len,packets)))+atom(b'stsc',word(0,1,1,count,1))+atom(b'stco',word(0,1,len(ftyp)+8))+atom(b'stss',word(0,1,1)))
    dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
    tkhd=bytearray(84);tkhd[12:16]=word(1);tkhd[20:24]=word(count);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000);tkhd[76:84]=word(size[0]<<16,size[1]<<16)
    mdhd=bytearray(24);mdhd[12:20]=word(50,count);mdhd[20:22]=(21956).to_bytes(2,'big')
    mdia=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'vide'+bytes(12))+atom(b'minf',atom(b'vmhd',word(1)+bytes(8))+dinf+stbl))
    mvhd=bytearray(100);mvhd[12:20]=word(50,count)
    return ftyp+mdat+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',atom(b'tkhd',tkhd)+mdia))
def main():
    data=(ROOT/'av1-spatial-operating-points.obu').read_bytes();units=[];seq=None
    for kind,sid,p in pieces(data):
        if kind==2:units.append([])
        assert units
        units[-1].append((kind,sid,p))
        if kind==1:seq=p
    assert len(units)==4
    records=[]
    for name,groups in [('complete',units),('missing-last-upper',units[:-1]+[[p for p in units[-1] if not(p[0]==6 and p[1]==1)] ]),('invalid-two-units',[units[0]+units[1]]),('invalid-repeated-layers',[units[0]+[p for p in units[1] if p[0]!=2]])]:
        packets=[b''.join(p for _,_,p in group) for group in groups]
        r={'name':name,'shown_indices':[1,3,5,7] if name=='complete' else [1,3,5,6] if name=='missing-last-upper' else [],'artifacts':[]}
        for ext,wrap in [('webm',webm),('mp4',mp4)]:
            output=wrap(packets,seq);file=f'av1-spatial-container-{name}.{ext}';(ROOT/file).write_bytes(output)
            r['artifacts'].append({'file':file,'sha256':hashlib.sha256(output).hexdigest()})
        records.append(r)
    (ROOT/'av1-spatial-container-generated.json').write_text(json.dumps({'fixtures':records},indent=2)+'\n')
if __name__=='__main__':main()
