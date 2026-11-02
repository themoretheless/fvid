"""Authored AVI G711 record windows and independent native float PCM oracles."""
from pathlib import Path
import struct,json,hashlib
from generate_g711_fixtures import level
from generate_avi_audio_eof_regressions import chunk,listing,stream,generate as neutral
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-g711-windows'
def generate():
    coded=frame(8,'420',16,16);files={'video.mkv':neutral()['video.mkv']};cases=[]
    for law,tag in [('alaw',6),('mulaw',7)]:
        for channels,rate in [(1,8000),(2,48000)]:
            for layout,parts,start in [('single',[8193],0),('single-delayed',[8193],73),('grouped',[1,8193,3],0),('grouped-delayed',[1,8193,3],73),('small',[1,2047,1],73)]:
                n=sum(parts);name=f'{law}-{channels}ch-{layout}';raw=bytes((i*17+(0 if c==0 else 91))%256 for i in range(n)for c in range(channels));gold=b''.join(struct.pack('<f',level(law,c)/32768.)for c in raw);fmt=struct.pack('<HHIIHHH',tag,channels,rate,rate*channels,channels,8,0)
                ah=bytearray(stream(b'auds',bytes(4),1,rate,n,channels));struct.pack_into('<I',ah,28,start)
                audio=listing(b'strl',chunk(b'strh',bytes(ah))+chunk(b'strf',fmt));vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
                video=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf));main=struct.pack('<14I',20000,0,0,0,1,0,2,max(len(coded),max(parts)*channels),16,16,0,0,0,0)
                records=chunk(b'00dc',coded);at=0
                for count in parts:records+=chunk(b'01wb',raw[at*channels:(at+count)*channels]);at+=count
                body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+video+audio)+listing(b'movi',records);files[name+'.avi']=b'RIFF'+struct.pack('<I',len(body))+body;files[name+'.f32le']=gold
                cases.append(dict(file=name+'.avi',oracle=name+'.f32le',law=law,channels=channels,rate=rate,frames=n,start=start,parts=parts))
    files['manifest.json']=(json.dumps(dict(provenance='20 pure-authored classic AVI A-law/mu-law programs,mono8k/stereo48k,8193-frame chunks and boundary/group/delayed controls. All256 codes and independent segment-level float PCM oracle. Authored neutral VP9 in AVI and Matroska;AVI video playback support is not asserted. No private input,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
