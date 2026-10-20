#!/usr/bin/env python3
"""Short authored VP9 clips with schema-default empty Matroska clocks."""
import hashlib,json
from pathlib import Path
from generate_matroska_header_stripping_regressions import clip
from generate_audio_resample_window_fixture import ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-empty-clock'
def rewrite(data,segment_scale=None,track_scale=None):
    out=bytearray();at=0
    while at<len(data):
        width=9-data[at].bit_length();tag=data[at:at+width];at+=width
        width=9-data[at].bit_length();length=int.from_bytes(data[at:at+width],'big')&((1<<(7*width))-1);at+=width
        payload=data[at:at+length];at+=length;ident=int.from_bytes(tag,'big')
        if ident in [0x18538067,0x1549a966,0x1654ae6b,0xae]:payload=rewrite(payload,segment_scale,track_scale)
        if ident==0x2ad7b1 and segment_scale is not None:payload=segment_scale
        if ident==0xae and track_scale is not None:payload+=ebml('23314f',track_scale)
        out+=ebml(tag.hex(),payload)
    return bytes(out)
def generate():
    base=clip();files={'control.mkv':base}
    for name,s,t in [('segment',b'',None),('track',None,b''),('both',b'',b''),('zero-segment',b'\0',None),('invalid-track-width',None,b'\1')]:files[name+'.mkv']=rewrite(base,s,t)
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three VP9 keyframes,55ms. Empty clock fields use schema defaults, explicit-zero and malformed-width cases are distinct refusals. No encoder, private media or network.','files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
