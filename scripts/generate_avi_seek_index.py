"""Short authored AVI with dense/irregular PCM and G711 physical record indices."""
from pathlib import Path
import struct,json,hashlib
from generate_g711_fixtures import level
from generate_avi_audio_eof_regressions import chunk,listing,stream,generate as neutral
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-seek-index'
def generate():
    coded=frame(8,'420',16,16);files={'video.mkv':neutral()['video.mkv']};cases=[]
    profiles=[('one',[1],0),('small',[1]*16,73),('medium',[1]*1024,0),('dense',[1]*65536,73),('irregular',[1,3,7,2]*4096,73)]
    for codec,tag,channels,bits in [('pcm16',1,1,16),('alaw',6,1,8),('mulaw',7,2,8)]:
        for layout,parts,start in profiles:
            n=sum(parts);stride=channels*(bits//8);rate=48000;name=f'{codec}-{layout}';raw=[];gold=[]
            for i in range(n):
                for c in range(channels):
                    value=(i*17%32768)-16384 if tag==1 else level(codec,(i*17+c*91)%256)
                    raw.append(struct.pack('<h',value)if tag==1 else bytes([(i*17+c*91)%256]));gold.append(struct.pack('<f',value/32768.))
            raw=b''.join(raw);gold=b''.join(gold);fmt=struct.pack('<HHIIHH',tag,channels,rate,rate*stride,stride,bits)+(bytes(2)if tag!=1 else b'');ah=bytearray(stream(b'auds',bytes(4),1,rate,n,stride));struct.pack_into('<I',ah,28,start)
            audio=listing(b'strl',chunk(b'strh',bytes(ah))+chunk(b'strf',fmt));vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0);video=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf));main=struct.pack('<14I',20000,0,0,0,1,0,2,max(len(coded),max(parts)*stride),16,16,0,0,0,0)
            records=[chunk(b'00dc',coded)];at=0
            for count in parts:records.append(chunk(b'01wb',raw[at*stride:(at+count)*stride]));at+=count
            body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+video+audio)+listing(b'movi',b''.join(records));files[name+'.avi']=b'RIFF'+struct.pack('<I',len(body))+body;files[name+'.f32le']=gold
            cases.append(dict(file=name+'.avi',oracle=name+'.f32le',codec=codec,channels=channels,rate=rate,frames=n,start=start,parts=parts,records=len(parts)))
    files['manifest.json']=(json.dumps(dict(provenance='15 pure authored VP9/AVI PCM16/A-law/mu-law programs with1/16/1024/65536 dense records or16384 irregular1/3/7/2-frame records. All audio is at48kHz and lasts at most1.366seconds. Independent exact float32 oracles,nonzero/zero starts and neutral playable Matroska. No private input,FFmpeg or network;AVI video playback support is not asserted.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
