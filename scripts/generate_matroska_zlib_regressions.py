#!/usr/bin/env python3
"""Authored VP9/Opus zlib transforms and independently classified damaged streams."""
import json,hashlib,zlib
from pathlib import Path
from generate_matroska_header_stripping_regressions import clip,opus_lace_clip
from generate_vp9_chroma import frame
from generate_audio_resample_window_fixture import ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-zlib'

def mutate_first_packet(data,transform):
    changed=False
    def rewrite(data):
        nonlocal changed
        out=bytearray();at=0
        while at<len(data):
            start=at;width=9-data[at].bit_length();tag=data[at:at+width];at+=width
            width=9-data[at].bit_length();length=int.from_bytes(data[at:at+width],'big')&((1<<(7*width))-1);at+=width
            payload=data[at:at+length];assert len(payload)==length;at+=length;ident=int.from_bytes(tag,'big')
            if ident in [0x18538067,0x1f43b675,0xa0]:payload=rewrite(payload)
            elif ident==0xa1 and not changed:
                payload=payload[:4]+transform(payload[4:]);changed=True
            else:out+=data[start:at];continue
            out+=ebml(tag.hex(),payload)
        return bytes(out)
    result=rewrite(data);assert changed;return result

def generate():
    files={'control.mkv':clip(),'laced-control.mkv':clip(laced=True),'opus-control.mkv':opus_lace_clip(False),'identical-control.mkv':clip(identical=True),'identical-zlib.mkv':clip(encoded=True,algorithm=0,identical=True)}
    cases=[]
    for name,arguments in [('plain',{}),('empty-scope',{'empty_scope':True}),('implicit-algorithm',{'omit_algorithm':True}),('implicit-encoding',{'implicit':True}),('late-tracks',{'late_tracks':True}),('laced',{'laced':True}),('laced-late',{'laced':True,'late_tracks':True})]:
        file=name+'.mkv';files[file]=clip(encoded=True,algorithm=0,**arguments)
        cases.append({'file':file,'control':'laced-control.mkv' if arguments.get('laced') else 'control.mkv'})
    files['opus-zlib.mkv']=opus_lace_clip(True,algorithm=0)
    cases.append({'file':'opus-zlib.mkv','control':'opus-control.mkv'})
    cases.append({'file':'identical-zlib.mkv','control':'identical-control.mkv'})
    source=files['plain.mkv'];invalid={}
    def fixed(payload):
        compressor=zlib.compressobj(strategy=zlib.Z_FIXED)
        return compressor.compress(zlib.decompress(payload))+compressor.flush()
    for name,transform in [('stored-deflate',lambda p:zlib.compress(zlib.decompress(p),0)),('fixed-deflate',fixed)]:
        file=name+'.mkv';files[file]=mutate_first_packet(source,transform);cases.append({'file':file,'control':'control.mkv'})
    changes=[('checksum',lambda p:p[:-1]+bytes([p[-1]^1]),'invalid or truncated Matroska zlib stream'),
             ('truncated',lambda p:p[:-1],'invalid or truncated Matroska zlib stream'),
             ('header',lambda p:b'\0'+p[1:],'invalid or truncated Matroska zlib stream'),
             ('trailing',lambda p:p+b'\0','trailing data after Matroska zlib stream'),
             ('empty',lambda p:zlib.compress(b''),'empty Matroska zlib frame'),
             ('output-budget',lambda p:zlib.compress(zlib.decompress(p)+bytes(16_384)),'WebM packet exceeds budget')]
    for name,transform,error in changes:
        file='invalid-'+name+'.mkv';files[file]=mutate_first_packet(source,transform);invalid[file]={'error':error,'packet_limit':16_384 if name=='output-budget' else 32<<20}
    original=frame(8,'420',16,16);files['frame.packet']=original;files['frame.zlib']=zlib.compress(original)
    return files,cases,invalid

def main():
    OUT.mkdir(parents=True,exist_ok=True);files,cases,invalid=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three VP9 keyframes and mono CELT silence, wrapped with Python standard-library zlib. No external encoder or private media.','cases':cases,'invalid':invalid,'files':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
