#!/usr/bin/env python3
"""RFC 8794 section 13 strings with ignored bytes after a NUL terminator."""
import json,hashlib
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_matroska_clock_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-text-termination'
MASTERS={0x1a45dfa3,0x18538067,0x1549a966,0x1654ae6b,0xae,0xe0}
def rewrite(data,replacements,name=None,additions=None):
    result=bytearray();at=0
    while at<len(data):
        start=at;width=9-data[at].bit_length();tag=data[at:at+width];at+=width
        width=9-data[at].bit_length();length=int.from_bytes(data[at:at+width],'big')&((1<<(7*width))-1);at+=width
        payload=data[at:at+length];assert len(payload)==length;at+=length;ident=int.from_bytes(tag,'big')
        if ident in MASTERS:
            payload=rewrite(payload,replacements,name,additions)
            payload+=(additions or {}).get(ident,b'')
            if ident==0xae and name is not None:payload+=ebml('536e',name)
        elif ident in replacements:payload=replacements[ident]
        else:result+=data[start:at];continue
        result+=ebml(tag.hex(),payload)
    return bytes(result)
def generate():
    source=clip()
    return {'control.mkv':source,
            'doctype-padding.mkv':rewrite(source,{0x4282:b'matroska\0\0'}),
            'doctype-suffix.mkv':rewrite(source,{0x4282:b'matroska\0\xff'}),
            'codec-suffix.mkv':rewrite(source,{0x86:b'V_VP9\0old'}),
            'codec-invalid-suffix.mkv':rewrite(source,{0x86:b'V_VP9\0\xff'}),
            'name-suffix.mkv':rewrite(source,{},'Кадр'.encode()+b'\0old'),
            'name-invalid-suffix.mkv':rewrite(source,{},'Кадр'.encode()+b'\0\xff'),
            'codec-invalid-prefix.mkv':rewrite(source,{0x86:b'\xff\0V_VP9'}),
            'metadata-suffix.mkv':rewrite(source,{0x5741:b'fvid\0\xff'},additions={0x18538067:ebml('1043a770',ebml('45b9',ebml('b6',ebml('73c4',b'\1')+ebml('91',b'\0')+ebml('80',ebml('85',b'Chapter\0\xff')))))})}
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three VP9 keyframes. Only EBML text fields change; no external encoder or private media.','standard':'https://www.rfc-editor.org/rfc/rfc8794.html#section-13','files':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n')
