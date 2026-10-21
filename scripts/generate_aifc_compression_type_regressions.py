#!/usr/bin/env python3
"""Author incomplete AIFC codec identifiers and independent endian PCM oracles."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as video
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aifc-compression-type'
def chunk(k,b):return k+struct.pack('>I',len(b))+b+bytes(len(b)%2)
def generate():
    files={'video.mkv':video()['video.mkv']};cases=[];controls=[]
    for bits,channels in [(16,1),(16,2),(24,1),(32,2)]:
        key=f'int{bits}-{channels}ch';values=[v*(1 if c==0 else -2)for v in [1000,-2000,3000,-4000,5000,-6000,7000,-8000,9000]for c in range(channels)]
        little=b''.join(v.to_bytes(bits//8,'little',signed=True)for v in values);big=b''.join(v.to_bytes(bits//8,'big',signed=True)for v in values)
        gold=b''.join(struct.pack('<f',v/(1<<(bits-1)))for v in values)
        wrong=b''.join(struct.pack('<f',int.from_bytes(little[i:i+bits//8],'big',signed=True)/(1<<(bits-1)))for i in range(0,len(little),bits//8));assert gold!=wrong
        files[key+'.f32le']=gold;files[key+'-wrong-big.f32le']=wrong
        header=struct.pack('>hIh',channels,9,bits)+bytes.fromhex('400bfa00000000000000')
        def build(form,comm,payload):
            body=form+(chunk(b'FVER',bytes.fromhex('a2805140'))if form==b'AIFC'else b'')+chunk(b'COMM',comm)+chunk(b'SSND',bytes(8)+payload)
            return b'FORM'+struct.pack('>I',len(body))+body
        for suffix,form,comm,payload in [('aiff',b'AIFF',header,big),('none',b'AIFC',header+b'NONE\0\0',big),('sowt',b'AIFC',header+b'sowt\0\0',little),('sowt-minimal',b'AIFC',header+b'sowt',little)]:
            n=key+'-'+suffix+'.aiff';files[n]=build(form,comm,payload);controls.append(dict(file=n,expected=key+'.f32le',bits=bits,channels=channels,frames=9,comm_bytes=len(comm)))
        for size in range(18,22):
            n=key+f'-short-{size}.aiff';files[n]=build(b'AIFC',(header+b'sowt')[:size],little);cases.append(dict(file=n,control=key+'-sowt.aiff',expected=key+'.f32le',wrong=key+'-wrong-big.f32le',bits=bits,channels=channels,frames=9,comm_bytes=size))
    files['manifest.json']=(json.dumps(dict(provenance='16 authored AIFC COMM18/19/20/21-byte incomplete sowt IDs;16 controls include plain AIFF,AIFC NONE/sowt with names and complete22-byte nameless sowt compatibility. Asymmetric manual integer16/24/32 mono/stereo PCM and independent big-endian reinterpretation oracle prove wrong decoder byte order. All FORM/chunk sizes,FVER,SSND offset and sample counts are authored correctly. Separate own VP9 companion. No private media,FFmpeg or network.',cases=cases,controls=controls,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
