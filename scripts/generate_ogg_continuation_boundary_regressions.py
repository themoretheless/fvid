#!/usr/bin/env python3
"""Author decoder-valid Opus padding that exposes Ogg continuation boundaries."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as owned
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ogg-continuation-boundary'
def sealed(payload,laces,granule,flags,sequence,serial=0x46564310):
    assert sum(laces)==len(payload)
    b=bytearray(b'OggS'+bytes([0,flags])+struct.pack('<qII',granule,serial,sequence)+bytes(4)+bytes([len(laces)])+bytes(laces)+payload)
    crc=0
    for v in b:
        crc^=v<<24
        for _ in range(8):crc=((crc<<1)^0x04c11db7 if crc&0x80000000 else crc<<1)&0xffffffff
    struct.pack_into('<I',b,22,crc);return bytes(b)
def padded(frames,padding):
    n=len(padding);length=[]
    while n>=254:length.append(255);n-=254
    length.append(n)
    return bytes([0xfb,0x40|frames])+bytes(length)+bytes.fromhex('fffe')*frames+padding

def generate():
    head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
    raw20=bytes.fromhex('f8fffe');raw10=bytes.fromhex('f0fffe')
    child=padded(1,bytes(250)+raw10);exact=padded(1,bytes(250))
    parent=padded(2,bytes(247)+child);parent510=padded(2,bytes(502))
    assert len(child)==258 and len(exact)==255 and len(parent)==513 and len(parent510)==510
    assert parent[255:]==child and parent[510:]==raw10
    packets={'raw20.packet':raw20,'raw10.packet':raw10,'padded20.packet':child,'exact255.packet':exact,'parent40.packet':parent,'parent510.packet':parent510,'empty.packet':b''}
    files={'video.mkv':owned()['video.mkv'],'mono.head':head,**packets};controls=[];malformed=[];joined=[]
    headers=sealed(head,[len(head)],0,2,0)+sealed(b'OpusTags'+bytes(8),[16],0,0,1)
    def add(name,pages,parts,kind='control',before=None,before_refusal=False,stream=True,ended=True):
        files[name+'.ogg']=headers+b''.join(sealed(p,l,g,f,i+2) for i,(p,l,g,f) in enumerate(pages))
        item=dict(file=name+'.ogg',parts=parts,ended=ended,stream_reader=stream)
        if before is not None:item['before_parts']=before
        if before_refusal:item['before_source_refusal']=True
        (controls if kind=='control' else malformed if kind=='malformed' else joined).append(item)
    p20=[dict(file='padded20.packet',frames=960)];r20=[dict(file='raw20.packet',frames=960)];r10=[dict(file='raw10.packet',frames=480)]
    full=[dict(file='parent40.packet',frames=1920),*r20]
    add('one-page-padded',[(child,[255,3],960,4)],p20)
    add('split-padded',[(child[:255],[255],-1,0),(child[255:],[3],960,5)],p20)
    add('completed-prefix',[(child[:255],[255],-1,0),(child[255:],[3],960,1)],p20,ended=False)
    add('zero-terminator',[(exact,[255],-1,0),(b'',[0],960,5)],[dict(file='exact255.packet',frames=960)])
    add('parent-one-page',[(parent+raw20,[255,255,3,3],2880,4)],full)
    add('parent-three-pages',[(parent[:255],[255],-1,0),(parent[255:510],[255],-1,1),(parent[510:]+raw20,[3,3],2880,5)],full)
    add('empty-bridge',[(child[:255],[255],-1,0),(b'',[],-1,0),(child[255:],[3],960,5)],p20,stream=False)
    add('plain-ten',[(raw10,[3],480,4)],r10)
    add('missing-final-prefix',[(child[:255],[255],-1,0),(child[255:],[3],960,0)],p20,kind='malformed',before=r10,ended=False)
    add('missing-middle',[(parent[:255],[255],-1,0),(parent[255:510],[255],-1,0),(parent[510:]+raw20,[3,3],2880,5)],full,kind='malformed',before=full)
    add('missing-zero-terminator-prefix',[(exact,[255],-1,0),(b'',[0],960,0)],[dict(file='exact255.packet',frames=960)],kind='malformed',before=[dict(file='empty.packet',frames=0)],before_refusal=True,ended=False)
    # These joined captures retain only a whole-page suffix of a parent packet.
    # Counters are resequenced coherently;the CONTINUES marker still forbids
    # interpreting packet-shaped bytes in the missing-prefix packet as audio.
    add('joined-tail',[(parent[510:]+raw20,[3,3],2880,5)],r20,kind='joined',before=[*r10,*r20])
    add('joined-two-pages',[(parent[255:510],[255],-1,1),(parent[510:]+raw20,[3,3],2880,5)],r20,kind='joined',before=[dict(file='padded20.packet',frames=960),*r20])
    add('joined-zero-terminator',[(raw20,[0,3],2880,5)],r20,kind='joined',before=[dict(file='empty.packet',frames=0),*r20],before_refusal=True)
    files['manifest.json']=(json.dumps(dict(provenance='Authored CRC-valid,consecutively numbered Ogg pages. Three malformed missing-continuation cases and three valid joined-live prefixes isolate fragment handling;packet-shaped bytes inside required-to-accept nonzero Opus padding must not become new audio. Eight complete/prefix controls cover255-byte nil termination,multiple pages and an empty-page compatibility case. Nonzero padding is decoder-acceptance coverage per RFC6716,not a conforming-encoder-output claim. All payloads and metadata are authored;owned VP9/PCM companion,no encoder/private media/network.',reference=['https://www.rfc-editor.org/rfc/rfc7845.html#section-3','https://www.rfc-editor.org/rfc/rfc6716.html#section-3.2.5'],controls=controls,malformed=malformed,joined=joined,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
