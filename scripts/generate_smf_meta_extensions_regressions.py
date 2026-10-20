"""Author extended SMF meta payloads and valid length-zero controls."""
import json,hashlib
from pathlib import Path
from generate_smf_independent_tracks_regressions import smf
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-meta-extensions'
def generate():
    note=b'\0\x90\x31\x40\x60\x80\x31\0';eot=b'\0\xff\x2f\0'
    tempo=b'\0\xff\x51\3\x03\xd0\x90'
    files={'video.mkv':pcm()['video.mkv'],'valid.mid':smf(0,[tempo+note+eot]),'valid-with-padding.mid':smf(0,[tempo+note+eot+b'\0'*8])};extended=[];short=[]
    for length in [0,1,2,3,127]:
        name=f'end-length-{length}.mid';files[name]=smf(0,[tempo+note+b'\0\xff\x2f'+bytes([length])+b'\0'*length]);extended.append(name)
    for length in [3,4,5,127]:
        name=f'tempo-length-{length}.mid';files[name]=smf(0,[b'\0\xff\x51'+bytes([length])+b'\x03\xd0\x90'+b'\0'*(length-3)+note+eot]);extended.append(name)
    for length in [0,1,2]:
        name=f'tempo-short-{length}.mid';files[name]=smf(0,[b'\0\xff\x51'+bytes([length])+b'\0'*length+note+eot]);short.append(name)
    files['manifest.json']=(json.dumps(dict(provenance='Authored known meta-event extensions: FF2Fend payload0/1/2/3/127andFF51tempo3/4/5/127compare independent canonical250000us tempo. Three too-short tempo payloads require refusal. Original SMFgeneric meta rule requires skipping unknown trailing fields. Synthetic note andVP9/PCM companion,no private media/encoder/network.',short=short,valid=['valid.mid','valid-with-padding.mid']+extended,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
