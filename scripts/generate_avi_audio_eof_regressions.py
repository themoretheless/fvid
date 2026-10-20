"""Author VP9/AVI with three tiny native ADPCM blocks, plus identical IVF video."""
import hashlib,json,struct
from pathlib import Path
from generate_vp9_chroma import frame
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-audio-eof'
def chunk(tag,data):return tag+struct.pack('<I',len(data))+data+bytes(len(data)%2)
def listing(kind,data):return chunk(b'LIST',kind+data)
def stream(kind,handler,scale,rate,length,size):
    return struct.pack('<4s4sIHHIIIIIIIIhhhh',kind,handler,0,0,0,0,scale,rate,0,length,size,0xffffffff,size if kind==b'auds' else 0,0,0,16 if kind==b'vids' else 0,16 if kind==b'vids' else 0)
def generate():
    coded=frame(8,'420',16,16)
    files={'video.ivf':struct.pack('<4sHH4sHHIIII',b'DKIF',0,32,b'VP90',16,16,50,1,1,0)+struct.pack('<IQ',len(coded),0)+coded}
    files['video.mkv']=clip(160,8000,1,packets=1)[0]
    assert coded in files['video.mkv']
    cases=[]
    for name,tag,frames in [('ima',17,9),('ms',2,4)]:
        extra=struct.pack('<H',frames)
        if tag==2:extra+=struct.pack('<H',7)+b''.join(struct.pack('<hh',a,b) for a,b in [(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)])
        fmt=struct.pack('<HHIIHHH',tag,1,8000,8*8000//frames,8,4,len(extra))+extra
        audio=listing(b'strl',chunk(b'strh',stream(b'auds',bytes(4),frames,8000,3,8))+chunk(b'strf',fmt))
        video_fmt=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
        video=listing(b'strl',chunk(b'strh',stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',video_fmt))
        main=struct.pack('<14I',20000,0,0,0,1,0,2,max(8,len(coded)),16,16,0,0,0,0)
        records=chunk(b'00dc',coded);expected=b''
        for predictor in [1000,2000,3000]:
            payload=struct.pack('<hBB',predictor,0,0)+bytes(4) if tag==17 else struct.pack('<BHhhB',0,16,predictor,predictor,0)
            assert len(payload)==8
            records+=chunk(b'01wb',payload);expected+=struct.pack('<f',predictor/32768.)*frames
        body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+video+audio)+listing(b'movi',records)
        files[name+'.avi']=b'RIFF'+struct.pack('<I',len(body))+body;files[name+'.f32le']=expected;cases.append({'name':name,'frames':frames*3,'block_frames':frames})
    files['manifest.json']=(json.dumps({'provenance':'Authored AVI with VP9 video and three8byte IMA/MS ADPCM constant-predictor blocks. Identical VP9 bitstream in Matroska verifies the video independently; IVF is also authored as a reference; AVI video playback support is not asserted. No encoder/private media/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
