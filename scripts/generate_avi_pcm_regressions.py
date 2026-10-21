#!/usr/bin/env python3
"""Author classic AVI integer/float PCM with independent WAV and f32 controls."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as owned,chunk,listing,stream
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-pcm'
def generate():
    coded=frame(8,'420',16,16);files={'video.mkv':owned()['video.mkv']};assert coded in files['video.mkv'];cases=[];malformed=[]
    def build(tag,bits,channels,n,parts,raw,partial=False,align_delta=0,avg_delta=0,scale=1,declared=None):
        width=channels*(bits//8);align=width+align_delta;fmt=struct.pack('<HHIIHH',tag,channels,8000,8000*width+avg_delta,align,bits)+(bytes(2) if tag==3 else b'')
        a=listing(b'strl',chunk(b'strh',stream(b'auds',bytes(4),scale,8000,n if declared is None else declared,max(parts)*width))+chunk(b'strf',fmt))
        # stream() uses its final size for dwSampleSize,which must be one frame.
        a=bytearray(a);pos=a.index(b'auds');struct.pack_into('<I',a,pos+44,width)
        vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
        v=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf))
        main=struct.pack('<14I',20000,0,0,0,1,0,2,max(len(coded),max(parts)*width),16,16,0,0,0,0)
        records=chunk(b'00dc',coded);at=0
        for index,count in enumerate(parts):
            data=raw[at*width:(at+count)*width];at+=count
            if partial and index+1==len(parts):data=data[:-1]
            records+=chunk(b'01wb',data)
        assert at==n
        body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+v+bytes(a))+listing(b'movi',records)
        avi=b'RIFF'+struct.pack('<I',len(body))+body
        wavebody=b'WAVE'+chunk(b'fmt ',fmt)+(chunk(b'fact',struct.pack('<I',n)) if tag==3 else b'')+chunk(b'data',raw)
        return avi,b'RIFF'+struct.pack('<I',len(wavebody))+wavebody
    profiles=[(1,b,c,9,[2,3,4])for b in [8,16,24,32]for c in [1,2]]+[(3,b,c,9,[2,3,4])for b in [32,64]for c in [1,2]]+[(3,32,2,8193,[8193])]
    pattern=[-.75,-.5,-.25,0.,.25,.5,.75,.125,-.125]
    for tag,bits,channels,n,parts in profiles:
        key=('int' if tag==1 else 'float')+str(bits)+f'-{channels}ch'+('-window' if n>9 else '')
        raw=b'';gold=b''
        for at in range(n):
            for channel in range(channels):
                value=pattern[at%len(pattern)]*(1 if channel==0 else -.5)
                if value==0:value=0.0
                if tag==3:data=struct.pack('<f' if bits==32 else '<d',value)
                elif bits==8:data=bytes([int(value*128)+128])
                else:data=int(value*(1<<(bits-1))).to_bytes(bits//8,'little',signed=True)
                raw+=data;gold+=struct.pack('<f',value)
        avi,wav=build(tag,bits,channels,n,parts,raw);files[key+'.avi']=avi;files[key+'.wav']=wav;files[key+'.f32le']=gold
        cases.append(dict(file=key+'.avi',control=key+'.wav',expected=key+'.f32le',tag=tag,bits=bits,channels=channels,frames=n,parts=parts,frame_bytes=channels*(bits//8)))
        if (tag,bits,channels,n) in [(1,16,2,9),(1,24,2,9),(3,32,1,9),(1,16,1,9),(3,64,2,9)]:
            kinds=[]
            if(tag,bits,channels)==(1,16,2):kinds=[('partial',dict(partial=True))]
            elif(tag,bits,channels)==(1,24,2):kinds=[('align',dict(align_delta=-1))]
            elif(tag,bits,channels)==(3,32,1):kinds=[('byte-rate',dict(avg_delta=1))]
            elif(tag,bits,channels)==(1,16,1):kinds=[('clock',dict(scale=2)),('unsupported-depth',{})]
            else:kinds=[('count',dict(declared=8))]
            for reason,kw in kinds:
                data=build(tag,bits,channels,n,parts,raw,**kw)[0]
                if reason=='unsupported-depth':
                    b=bytearray(data);audio=b.index(b'auds');fmt_at=b.index(b'strf',audio)+8;struct.pack_into('<H',b,fmt_at+14,12);data=bytes(b)
                name=key+'-bad-'+reason+'.avi';files[name]=data;malformed.append(dict(file=name,reason=reason))
    files['manifest.json']=(json.dumps(dict(provenance='13 authored classic AVI PCM programs:unsigned8,signed16/24/32,float32/64 mono/stereo plus8193-frame bounded-window control. Independent WAV wrappers and literal dyadic f32 PCM;6 malformed/unsupported controls. All AVI contain owned VP9 with separate Matroska control. Pure Python,no private data,encoder or network. No WAVEFORMATEXTENSIBLE or performance claim.',cases=cases,malformed=malformed,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
