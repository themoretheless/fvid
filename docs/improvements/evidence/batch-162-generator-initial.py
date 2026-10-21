#!/usr/bin/env python3
"""Author CRC-valid Ogg page loss that an Opus granule origin can conceal."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as owned
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ogg-page-sequence'
def page(payload,granule,flags,sequence,serial):
    laces=bytes([255]*(len(payload)//255)+[len(payload)%255])
    b=bytearray(b'OggS'+bytes([0,flags])+struct.pack('<qII',granule,serial,sequence&0xffffffff)+bytes(4)+bytes([len(laces)])+laces+payload)
    crc=0
    for v in b:
        crc^=v<<24
        for _ in range(8):crc=((crc<<1)^0x04c11db7 if crc&0x80000000 else crc<<1)&0xffffffff
    struct.pack_into('<I',b,22,crc);return bytes(b)
def pages(serial=0x46565310,base=0,origin=0,count=3,sequences=None):
    head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
    specs=[(head,0,2),(b'OpusTags'+bytes(8),0,0)]+[(bytes.fromhex('f8fffe'),origin+(i+1)*960,4 if i==count-1 else 0) for i in range(count)]
    if sequences is None:sequences=range(base,base+len(specs))
    return [page(p,g,f,n,serial) for (p,g,f),n in zip(specs,sequences)]
def generate():
    files={'video.mkv':owned()['video.mkv']};cases=[];controls=[]
    def control(name,data,frames,chains=1,packets=5,native=True):
        files[name+'.ogg']=data;controls.append(dict(file=name+'.ogg',frames=frames,chains=chains,packets=packets,native=native))
    control('complete-control',b''.join(pages()),2880)
    control('positive-origin-control',b''.join(pages(origin=960,count=2)),1920,packets=4)
    control('legacy-counter-control',b''.join(pages(base=100)),2880)
    control('wrapped-counter-control',b''.join(pages(base=0xfffffffe)),2880)
    control('chained-control',b''.join(pages()+pages(serial=0x46565311)),5760,chains=2,packets=10)
    a=pages();b=pages(serial=0x46565311)
    control('interleaved-control',b''.join(x for pair in zip(a,b) for x in pair),2880,chains=2,packets=10,native=False)
    bad=[('lost-first-audio',a[:2]+a[3:],1920),('gap',pages(sequences=[0,1,3,4,5]),2880),('duplicate',pages(sequences=[0,1,2,2,4]),2880),('backward',pages(sequences=[0,1,2,1,4]),2880)]
    for name,data,before_frames in bad:
        files[name+'.ogg']=b''.join(data);cases.append(dict(file=name+'.ogg',before_frames=before_frames,packets=len(data),loss=name=='lost-first-audio'))
    files['manifest.json']=(json.dumps(dict(provenance='Four authored CRC-valid Ogg/Opus counter failures include an actually missing first audio page:the remaining granules mimic a legitimate positive origin and hide20ms loss. Six complete controls cover legal positive origin,chaining,interleaving,and existing-reader arbitrary initial/wrapping counter compatibility. Standard authored mono20ms DTX packets and owned VP9/PCM companion. No encoder,private media or network.',cases=cases,controls=controls,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
