"""Author MACE/GSM/TrueSpeech runs crossing nondivisible2048sample windows."""
import json,struct,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import chunk,generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/coded-window-clock'
def wave(tag,channels,rate,bits,block,byte_rate,payload):
    fmt=struct.pack('<HHIIHH',tag,channels,rate,byte_rate,block,bits)
    body=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',payload);return b'RIFF'+struct.pack('<I',len(body))+body
def gsm_block(seed):
    bits=[]
    def put(value,width):bits.extend((value>>i)&1 for i in range(width))
    for frame in range(2):
        for width in [6,6,5,5,4,4,3,3]:put(1<<(width-1),width)
        for sf in range(4):
            put(40+seed%40,7);put(seed%2,2);put((seed+sf)%4,2);put(20+seed%8,6)
            for i in range(13):put((seed+frame+sf+i)%8,3)
    assert len(bits)==520
    return bytes(sum(bits[i+j]<<j for j in range(8))for i in range(0,len(bits),8))
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for kind,width in [('MAC3',2),('MAC6',1)]:
        for channels in [1,2]:
            blocks=1024;payload=bytes((i*37+11)%256 for i in range(blocks*width*channels));comm=struct.pack('>hIh',channels,blocks*6,8)+bytes.fromhex('400bfa00000000000000')+kind.encode()+bytes(2)
            body=b'AIFC'+chunk(b'FVER',bytes.fromhex('a2805140'),True)+chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+payload,True);name=f'{kind.lower()}-{channels}';files[name+'.aiff']=b'FORM'+struct.pack('>I',len(body))+body;cases.append(dict(name=name,kind='aiff',file=name+'.aiff',frames=blocks*6,channels=channels,grain=2046))
    gsm=b''.join(gsm_block(i)for i in range(14));files['gsm.wav']=wave(0x31,1,8000,0,65,1625,gsm);cases.append(dict(name='gsm',kind='wav',file='gsm.wav',frames=4480,channels=1,grain=1920))
    speech=bytes((i*13+7)%256 for i in range(20*32));files['truespeech.wav']=wave(0x22,1,8000,1,32,1066,speech);cases.append(dict(name='truespeech',kind='wav',file='truespeech.wav',frames=4800,channels=1,grain=1920))
    files['manifest.json']=(json.dumps(dict(provenance='Fully authored short coded runs, no copied speech/media/codec parameters. MACE6sample units, Microsoft GSM320sample blocks with valid field widths, and32byte/240sample TrueSpeech blocks cross nondivisible2048frame windows. Continuous grouped native decode is the PCM consistency control. Short native VP9/PCM video companion. No encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
