"""Author20ms WAV/AIFF/AU PCM plus matching short VP9/PCM video control."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-reader-eof'
def chunk(tag,data,big=False):return tag+struct.pack('>I' if big else '<I',len(data))+data+bytes(len(data)%2)
def generate():
    values=[i%128-64 for i in range(160)];expected=b''.join(struct.pack('<f',v/128.) for v in values);files={'expected.f32le':expected};cases=[]
    pcm16=b''.join((v*256).to_bytes(2,'little',signed=True) for v in values)
    files['video.mkv']=clip(160,8000,1,packets=1,payload_transform=lambda _:pcm16)[0]
    for container in ['wav','aiff','au']:
        for bits,floating in [(8,False),(16,False),(24,False),(32,False),(32,True)]:
            width=bits//8;little=container=='wav';name=container+'-'+('float' if floating else 'int')+str(bits)
            if floating:payload=b''.join(struct.pack('<f' if little else '>f',v/128.) for v in values)
            elif bits==8 and little:payload=bytes(v+128 for v in values)
            else:payload=b''.join((v*(1<<(bits-8))).to_bytes(width,'little' if little else 'big',signed=True) for v in values)
            if container=='wav':
                body=b'WAVE'+chunk(b'fmt ',struct.pack('<HHIIHH',3 if floating else 1,1,8000,8000*width,width,bits))+chunk(b'data',payload);media=b'RIFF'+struct.pack('<I',len(body))+body
            elif container=='aiff':
                form=b'AIFC' if floating else b'AIFF';comm=struct.pack('>hIh',1,160,bits)+bytes.fromhex('400bfa00000000000000')
                if floating:comm+=b'fl32\0\0'
                body=form+(chunk(b'FVER',bytes.fromhex('a2805140'),True) if floating else b'')+chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+payload,True);media=b'FORM'+struct.pack('>I',len(body))+body
            else:media=struct.pack('>6I',0x2e736e64,24,len(payload),6 if floating else {8:2,16:3,24:4,32:5}[bits],8000,1)+payload
            files[name+'.'+container]=media;cases.append({'name':name,'format':container,'file':name+'.'+container,'bits':bits,'float':floating})
    files['manifest.json']=(json.dumps({'provenance':'Authored160PCM frames at8k mono in WAV/AIFF/AU integer8/16/24/32 and float32. Matching16bit PCM accompanies a short native VP9 video. No encoder/private media/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
