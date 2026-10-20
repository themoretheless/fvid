#!/usr/bin/env python3
"""Authored VP9 video plus 8kHz mono integer PCM; no external codecs."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-audio-defaults'
def clip(rate,channels,output=None):
    video=ebml('ae',ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(20_000_000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
    settings=ebml('6264',b'\x10')
    if rate is not None:settings+=ebml('b5',rate)
    if channels is not None:settings+=ebml('9f',channels)
    if output is not None:settings+=ebml('78b5',output if isinstance(output,bytes) else struct.pack('>d',output))
    audio=ebml('ae',ebml('d7',b'\2')+ebml('83',b'\2')+ebml('86',b'A_PCM/INT/LIT')+ebml('e1',settings))
    values=[(i*431)%24001-12000 for i in range(480)]
    blocks=ebml('e7',b'\0')
    for i in range(3):
        blocks+=ebml('a3',b'\x81'+struct.pack('>h',i*20)+b'\x80'+frame(8,'420',16,16))
        blocks+=ebml('a3',b'\x82'+struct.pack('>h',i*20)+b'\x80'+struct.pack('<160h',*values[i*160:(i+1)*160]))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60)))
    return ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))+ebml('18538067',info+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
def generate():
    rate=struct.pack('>d',8000)
    configurations={'control':(rate,b'\1'),'missing-rate':(None,b'\1'),'missing-channels':(rate,None),'missing-both':(None,None),'empty-rate':(b'',b'\1'),'empty-channels':(rate,b''),'empty-both':(b'',b''),'zero-rate':(struct.pack('>d',0),b'\1'),'zero-channels':(rate,b'\0')}
    files={name+'.mkv':clip(*args) for name,args in configurations.items()}
    files['empty-output-rate.mkv']=clip(rate,b'\1',b'')
    files['expected.f32le']=b''.join(struct.pack('<f',((i*431)%24001-12000)/32768) for i in range(480))
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Three authored VP9 keyframes and 480 authored mono PCM samples. 60ms video, RFC9559 audio defaults. No private media, external encoder or network.','files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
