"""Author tiny VP9/MS-ADPCM AVI files at exact sample-duration boundaries."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import chunk,listing,stream
from generate_vp9_chroma import frame
from generate_audio_reader_eof_regressions import generate as companion
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-duration-grid'
def generate():
    files={'video.mkv':companion()['video.mkv']};cases=[];coded=frame(8,'420',16,16)
    for rate,count in [(44100,384),(44100,192),(48000,64),(48000,128),(96000,64),(8000,384)]:
        levels=[4096,-4096,8192];samples=[levels[(i//64)%3] for i in range(count)]
        pcm=b''.join(struct.pack('<h',s) for s in samples)
        extra=struct.pack('<HH',64,7)+b''.join(struct.pack('<hh',a,b) for a,b in [(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)])
        fmt=struct.pack('<HHIIHHH',2,1,rate,rate*38//64,38,4,len(extra))+extra
        pcm=b''.join(struct.pack('<BHhh',0,16,levels[i%3],levels[i%3])+bytes(31) for i in range(count//64))
        audio=listing(b'strl',chunk(b'strh',stream(b'auds',bytes(4),64,rate,count//64,38))+chunk(b'strf',fmt))
        vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
        video=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf))
        main=struct.pack('<14I',20000,0,0,0,1,0,2,len(pcm),16,16,0,0,0,0)
        body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+video+audio)+listing(b'movi',chunk(b'00dc',coded)+b''.join(chunk(b'01wb',pcm[i:i+38]) for i in range(0,len(pcm),38)))
        name=f'ms-{rate}-{count}.avi';files[name]=b'RIFF'+struct.pack('<I',len(body))+body
        expected=name+'.f32le';files[expected]=b''.join(struct.pack('<f',s/32768) for s in samples)
        cases.append(dict(file=name,expected=expected,rate=rate,samples=count,end_ns=count*1000000000//rate))
    files['manifest.json']=(json.dumps(dict(provenance='Six authored VP9/MS-ADPCM AVI sample-clock boundaries, independent literal PCM. Separate native Matroska video companion; AVI video playback not claimed. No encoder/network/private data.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
