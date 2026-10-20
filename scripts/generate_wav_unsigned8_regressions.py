"""Author unsigned8 PCM Wave acceptance, signed16 controls and short video."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wav-unsigned8'
def chunk(tag,data):return tag+struct.pack('<I',len(data))+data+bytes(len(data)%2)
def wav(rate,channels,bits,payload,tag=1,extensible=False,valid_bits=None):
    fmt=struct.pack('<HHIIHH',0xfffe if extensible else tag,channels,rate,rate*channels*(bits//8),channels*(bits//8),bits)
    if extensible:fmt+=struct.pack('<HHI',22,bits if valid_bits is None else valid_bits,(1<<channels)-1)+struct.pack('<IHH8s',tag,0,0x10,bytes.fromhex('800000aa00389b71'))
    body=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',payload);return b'RIFF'+struct.pack('<I',len(body))+body

def generate():
    files={};cases=[]
    for name,rate,channels,extensible in [('mono',8000,1,False),('stereo',44100,2,False),('three-channel',48000,3,False),('extensible',8000,2,True)]:
        frames=rate//50;edge=[0,1,127,128,129,254,255]
        data=bytes(edge[(i+c)%len(edge)] if i<len(edge) else (i*37+c*67)%256 for i in range(frames) for c in range(channels))
        signed=b''.join(((v-128)*256).to_bytes(2,'little',signed=True) for v in data)
        files[name+'.wav']=wav(rate,channels,8,data,extensible=extensible);files[name+'-control.wav']=wav(rate,channels,16,signed)
        files[name+'.f32le']=b''.join(struct.pack('<f',(v-128)/128.) for v in data)
        files[name+'-video.mkv']=clip(frames,rate,1,packets=1,channels=channels,payload_transform=lambda _,data=signed:data)[0]
        cases.append({'name':name,'rate':rate,'channels':channels,'frames':frames})
    files['unsupported-float8.wav']=wav(8000,1,8,bytes(160),tag=3)
    files['manifest.json']=(json.dumps({'provenance':'Authored20ms unsigned8 PCM Wave mono/stereo/three-channel/extensible with explicit0/1/127/128/129/254/255 boundaries. Signed16 controls and matching VP9/PCM clips. No encoder/private media/network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
