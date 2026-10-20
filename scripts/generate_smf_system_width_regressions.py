"""Author notes separated by zero-operand MIDI system messages."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-system-width'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,message in [('control',b''),('tune-request',b'\0\xf6'),('continue',b'\0\xfb'),('stop',b'\0\xfc'),('clock',b'\0\xf8'),('start',b'\0\xfa'),('active-sensing',b'\0\xfe'),('song-position',b'\0\xf2\x01\x02'),('song-select',b'\0\xf3\x09')]:
        data=midi(b'\0\x90\x45\x40'+message+b'\x60\x80\x45\0\0\xff\x2f\0',480);files[name+'.mid']=data;cases.append(dict(name=name,file=name+'.mid'))
    files['manifest.json']=(json.dumps(dict(provenance='Authored100ms note with isolated MIDI system message width probes and correct one/two-byte controls. These adapter-supported messages are skipped; their transport/device meaning is not implemented. Short native VP9/PCM video. No private input/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
