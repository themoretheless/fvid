"""Shared MP4 box helpers and fixture output path; FFmpeg authoring lives in benchmark_ac3_container_priming_reference.py."""
import struct
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ac3-container-priming'
def boxes(data):
    at=0
    while at<len(data):
        size,kind=struct.unpack_from('>I4s',data,at);assert size>=8 and at+size<=len(data)
        yield kind,data[at+8:at+size];at+=size
    assert at==len(data)
def box(kind,data):return struct.pack('>I4s',len(data)+8,kind)+data
def repeated_audio_edits(data):
    outer=list(boxes(data));kinds=[k for k,_ in outer];assert kinds.index(b'moov')>kinds.index(b'mdat')
    rebuilt=[]
    for kind,body in outer:
        if kind==b'moov':
            children=list(boxes(body));movie=next(b for k,b in children if k==b'mvhd');assert movie[0]==0;scale=struct.unpack_from('>I',movie,12)[0];assert scale%1000==0
            modified=[]
            for k,b in children:
                if k==b'trak':
                    parts=list(boxes(b));mdia=next(v for t,v in parts if t==b'mdia');handler=next(v for t,v in boxes(mdia) if t==b'hdlr')
                    if handler[8:12]==b'soun':
                        entries=[(20*scale//1000,-1),(40*scale//1000,256),(40*scale//1000,256)]
                        edits=box(b'elst',bytes(4)+struct.pack('>I',3)+b''.join(struct.pack('>IiI',duration,start,65536)for duration,start in entries))
                        track=[]
                        for t,v in parts:
                            if t==b'edts':continue
                            if t==b'tkhd':v=bytearray(v);assert v[0]==0;struct.pack_into('>I',v,20,100*scale//1000);v=bytes(v)
                            track.append(box(t,v))
                        b=b''.join(track)+box(b'edts',edits)
                modified.append(box(k,b))
            body=b''.join(modified)
        rebuilt.append(box(kind,body))
    return b''.join(rebuilt)
