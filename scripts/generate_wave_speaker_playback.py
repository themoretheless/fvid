"""Invent short WAVE speaker probes; no encoder, private media or network."""
from pathlib import Path
import struct,json
from generate_mp4_seek_io_regressions import generate as video
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wave-speaker-playback'
def chunk(tag,data):return tag+struct.pack('<I',len(data))+data+bytes(len(data)%2)
def generate():
    v=video();files={'neutral-video.mp4':v['three-colors.mp4'],**{f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)}};cases=[]
    for coding in ['s16','f32']:
        for name,channels,mask in [('surround',6,63),('rear',2,48),('lfe',1,8),('front',3,7),('direct',3,0),('discrete',9,0),('partial',3,3),('classic',2,None)]:
            rate=44100;frames=4099;tag,bits=(1,16)if coding=='s16'else(3,32);block=channels*bits//8
            values=[(((t*37+i*311)%2048)-1024)/32768 for t in range(frames)for i in range(channels)]
            raw=b''.join(struct.pack('<h',int(x*32768))if coding=='s16'else struct.pack('<f',x)for x in values)
            fmt=struct.pack('<HHIIHH',65534 if mask is not None else tag,channels,rate,rate*block,block,bits)
            if mask is not None:fmt+=struct.pack('<HHII',22,bits,mask,tag)+bytes.fromhex('00001000800000aa00389b71')
            body=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',raw);key=coding+'-'+name;files[key+'.wav']=b'RIFF'+struct.pack('<I',len(body))+body;files[key+'.f32le']=b''.join(struct.pack('<f',x)for x in values)
            cases.append(dict(name=key,channels=channels,mask=mask,rate=rate,frames=frames))
    values=[(((t*37+i*311)%2048)-1024)/32768 for t in range(4099)for i in range(2)]
    pcm16=b''.join(struct.pack('>h',int(x*32768))for x in values)
    pcm32=b''.join(struct.pack('>f',x)for x in values)
    def bechunk(tag,data):return tag+struct.pack('>I',len(data))+data+bytes(len(data)%2)
    body=b'AIFF'+bechunk(b'COMM',struct.pack('>HIH',2,4099,16)+bytes.fromhex('400eac44000000000000'))+bechunk(b'SSND',bytes(8)+pcm16)
    files['precise-s16.aiff']=b'FORM'+struct.pack('>I',len(body))+body
    for name,tag,data in [('precise-s16.au',3,pcm16),('precise-f32.au',6,pcm32)]:files[name]=struct.pack('>6I',0x2e736e64,24,len(data),tag,44100,2)+data
    files['precise.f32le']=b''.join(struct.pack('<f',x)for x in values)
    # Authored invalid declaration, independent of successful routing cases.
    body=b'WAVE'+chunk(b'fmt ',struct.pack('<HHIIHHHHII',65534,1,44100,88200,2,16,22,16,3,1)+bytes.fromhex('00001000800000aa00389b71'))+chunk(b'data',bytes(8));files['too-many-speakers.wav']=b'RIFF'+struct.pack('<I',len(body))+body
    files['manifest.json']=(json.dumps(dict(provenance='Analytic interleaved exact binary PCM, handcrafted RIFF headers and authored VP9 control. No encoder, private media or network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
