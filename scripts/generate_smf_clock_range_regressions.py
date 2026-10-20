"""Author compact sparse MIDI clock arithmetic probes, plus short video control."""
import json,struct,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-clock-range'
def midi(track,ppq):return b'MThd'+struct.pack('>IHHH',6,0,1,ppq)+b'MTrk'+struct.pack('>I',len(track))+track
def generate():
    delta=0xfffffff;tempo=0xffffff;count=4097
    prefix=b'\0\xff\x51\x03\xff\xff\xff'+(b'\xff\xff\xff\x7f\xff\x01\0'*count)
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,track,ppq,expected in [('wide-product',prefix+b'\0\xff\x2f\0',32767,delta*count*tempo//32767),('wide-segment',prefix+b'\0\xff\x51\x03\x07\xa1\x20'+b'\x60\xff\x2f\0',32767,(delta*count*tempo+96*500000)//32767),('out-of-range',prefix+b'\0\xff\x2f\0',1,None),('control',b'\0\x90\x45\x64\x60\x80\x45\0\0\xff\x2f\0',480,100000)]:
        files[name+'.mid']=midi(track,ppq);cases.append(dict(name=name,file=name+'.mid',microseconds=expected))
    files['manifest.json']=(json.dumps(dict(provenance='Authored sparse MIDI VLQ/tempo arithmetic probes. Long claims are parser-only; no huge timeline is rendered. Normal100ms note and short native VP9/PCM companion provide playback controls. No private source/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
