"""Author fixed-frame seek controls across PCM widths and packet boundaries."""
import json,struct,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as short,chunk
from generate_wav_unsigned8_regressions import wav
from generate_audio_ring_capacity_regressions import clip
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/pcm-exact-seek'
def encode(kind,values,bits=16,floating=False):
    width=bits//8
    payload=b''.join(struct.pack(('<'if kind=='wav'else'>')+('d'if bits==64 else'f'),v/32768.)if floating else struct.pack('<h'if kind=='wav'else'>h',v)for v in values)
    if kind=='wav':return wav(8000,1,bits,payload,tag=3 if floating else 1)
    if kind=='au':return struct.pack('>6I',0x2e736e64,24,len(payload),7 if bits==64 else 6 if floating else 3,8000,1)+payload
    comm=struct.pack('>hIh',1,len(values),bits)+bytes.fromhex('400bfa00000000000000')
    if floating:comm+=b'fl64\0\0'
    body=(b'AIFC'if floating else b'AIFF')+(chunk(b'FVER',bytes.fromhex('a2805140'),True)if floating else b'')+chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+payload,True)
    return b'FORM'+struct.pack('>I',len(body))+body
def generate():
    p=short();manifest=json.loads(p['manifest.json']);files={'video.mkv':p['video.mkv'],'short.f32le':p['expected.f32le']};cases=[]
    for c in manifest['cases']:cases.append(dict(name=c['name'],kind=c['format'],source='playback-errors/audio-reader-eof/'+c['file'],expected='short.f32le',frames=160))
    values=[(i%128-64)*256 for i in range(160)]
    for kind in ['wav','aiff','au']:
        name=kind+'-float64';files[name+'.'+kind]=encode(kind,values,64,True);cases.append(dict(name=name,kind=kind,file=name+'.'+kind,expected='short.f32le',frames=160))
    values=[((i%251)-125)*128 for i in range(6147)];files['long.f32le']=b''.join(struct.pack('<f',v/32768.)for v in values)
    for kind in ['wav','aiff','au']:
        name=kind+'-long-int16';files[name+'.'+kind]=encode(kind,values);cases.append(dict(name=name,kind=kind,file=name+'.'+kind,expected='long.f32le',frames=6147))
    payload=b''.join(struct.pack('<h',v)for v in values);files['long-video.mkv']=clip(6147,8000,1,packets=1,payload_transform=lambda _:payload)[0]
    files['manifest.json']=(json.dumps(dict(provenance='Authored mono8k PCM integer8/16/24/32 and float32/64 WAV/AIFF/AU. Existing20ms authored references read in place;6147frame numeric ramps cross three2048frame packet boundaries. Short native VP9/PCM companions. No private input/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
