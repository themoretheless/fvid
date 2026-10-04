"""Join public synthetic AVC and AAC tracks without encoders or private media."""
from pathlib import Path
import struct
root=Path(__file__).parent
video=(root/'shared-avc-baseline.mp4').read_bytes()
audio=(root.parent/'audio/aac-native-edit.m4a').read_bytes()
def boxes(data):
    pos=0
    while pos<len(data):
        size=int.from_bytes(data[pos:pos+4],'big'); header=8
        if size==1: size=int.from_bytes(data[pos+8:pos+16],'big'); header=16
        elif size==0: size=len(data)-pos
        assert size>=header and pos+size<=len(data)
        yield data[pos+4:pos+8],data[pos+header:pos+size],pos,size
        pos+=size
def box(kind,payload):return struct.pack('>I',8+len(payload))+kind+payload
def moov(data):return next(payload for kind,payload,_,_ in boxes(data) if kind==b'moov')
def track(payload,shift,identifier):
    out=b''
    for kind,body,_,_ in boxes(payload):
        if kind in [b'mdia',b'minf',b'stbl']:body=track(body,shift,identifier)
        elif kind==b'edts' and identifier==2:
            # This regression uses the complete AAC packet timeline, not the
            # source's unrelated edit window/movie clock.
            kind=b'free'
        elif kind==b'tkhd':
            body=bytearray(body); at=20 if body[0]==1 else 12
            body[at:at+4]=struct.pack('>I',identifier);body=bytes(body)
        elif kind in [b'stco',b'co64']:
            body=bytearray(body);width=4 if kind==b'stco' else 8
            for at in range(8,len(body),width):
                body[at:at+width]=(int.from_bytes(body[at:at+width],'big')+shift).to_bytes(width,'big')
            body=bytes(body)
        out+=box(kind,body)
    return out
def without_moov(data):
    output=bytearray(data)
    for kind,_,at,size in boxes(data):
        if kind==b'moov':output[at+4:at+8]=b'free'
    return bytes(output)
vmoov=moov(video);amoov=moov(audio)
mvhd=bytearray(next(body for kind,body,_,_ in boxes(vmoov) if kind==b'mvhd'))
mvhd[-4:]=struct.pack('>I',3)
vtrack=next(body for kind,body,_,_ in boxes(vmoov) if kind==b'trak')
atrack=next(body for kind,body,_,_ in boxes(amoov) if kind==b'trak')
output=without_moov(video)+without_moov(audio)+box(b'moov',box(b'mvhd',mvhd)+box(b'trak',track(vtrack,0,1))+box(b'trak',track(atrack,len(video),2)))
(root/'shared-mp4-av.mp4').write_bytes(output)
