"""Derive CRC corruption from one existing authored32ms AC3 tone frame."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-crc'
def crc(data):
    c=0
    for v in data:
        c^=v<<8
        for _ in range(8):c=((c<<1)^(0x8005 if c&0x8000 else 0))&65535
    return c
def generate():
    source=(ROOT/'tests/fixtures/audio/ac3-stereo.ac3').read_bytes();frame=source[:len(source)//8];assert len(frame)==768 and crc(frame[2:])==0
    files={'video.mkv':pcm()['video.mkv'],'control.ac3':frame};cases=[]
    for name,index in [('crc1',2),('crc2',767),('payload',512)]:
        bad=bytearray(frame);bad[index]^=1;assert crc(bad[2:])!=0;files[name+'.ac3']=bytes(bad);cases.append(dict(name=name,file=name+'.ac3',offset=index))
    files['valid-then-bad.ac3']=frame+files['crc2.ac3']
    files['manifest.json']=(json.dumps(dict(provenance='One existing authored32ms synthetic stereo tone AC3 frame, valid full-frame CRC, with isolated CRC1/CRC2/payload bit flips. Invalid frames must use existing silence-concealment policy, not decoded corrupt PCM. Short authored VP9/PCM companion. No private input/encoder/network.',source_sha256=hashlib.sha256(source).hexdigest(),standard='https://www.atsc.org/wp-content/uploads/2015/03/A52-2018.pdf section7.10.1',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
