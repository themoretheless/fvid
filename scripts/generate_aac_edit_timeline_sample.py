#!/usr/bin/env python3
"""Patch synthetic AAC with a leading gap and repeated media range; no FFmpeg."""
from pathlib import Path
import struct
import sys
media_start=int(sys.argv[sys.argv.index("--media-start-ticks")+1]) if "--media-start-ticks" in sys.argv else 0
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
data=(root/'aac-no-edit.m4a').read_bytes()
def boxes(data):
    at=0
    while at<len(data):
        size,kind=struct.unpack_from('>I4s',data,at)
        assert size>=8 and at+size<=len(data)
        yield kind,data[at+8:at+size]
        at+=size
    assert at==len(data)
def box(kind,payload):return struct.pack('>I4s',len(payload)+8,kind)+payload
outer=list(boxes(data)); kinds=[k for k,_ in outer]
assert kinds.index(b'moov')>kinds.index(b'mdat') # chunk byte offsets stay fixed
out=[]
for kind,payload in outer:
    if kind==b'moov':
        children=list(boxes(payload));mvhd=next(p for k,p in children if k==b'mvhd');assert mvhd[0]==0
        scale=struct.unpack_from('>I',mvhd,12)[0];assert scale%1000==0
        entries=[(20*scale//1000,-1),(100*scale//1000,media_start),(100*scale//1000,media_start)]
        elst=box(b'elst',bytes(4)+struct.pack('>I',len(entries))+b''.join(struct.pack('>IiI',duration,start,65536) for duration,start in entries))
        rebuilt=[]
        for child,body in children:
            if child==b'trak':
                parts=[(k,p) for k,p in boxes(body) if k!=b'edts']
                body=b''.join(box(k,p) for k,p in parts)+box(b'edts',elst)
            rebuilt.append(box(child,body))
        payload=b''.join(rebuilt)
    out.append(box(kind,payload))
(root/('aac-gap-repeat-offset.m4a' if media_start else 'aac-gap-repeat.m4a')).write_bytes(b''.join(out))
