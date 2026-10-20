"""Author running-status cancellation and explicit status recovery controls."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-running-reset'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    barriers={'text':b'\xff\x01\x01x','tempo':b'\xff\x51\x03\x07\xa1\x20','unknown-meta':b'\xff\x09\0','sysex':b'\xf0\x02\x7d\xf7','escape':b'\xf7\x01\xf8'}
    for name,barrier in barriers.items():
        for bad in [False,True]:
            key=name+('-missing-status' if bad else '-explicit-status')
            track=b'\0\x90\x45\x40\0'+barrier+b'\x60'+(b'' if bad else b'\x90')+b'\x45\0\0\xff\x2f\0'
            files[key+'.mid']=midi(track,480);cases.append(dict(name=key,file=key+'.mid',bad=bad))
    files['ordinary-running.mid']=midi(b'\0\x90\x45\x40\x60\x45\0\0\xff\x2f\0',480)
    cases.append(dict(name='ordinary-running',file='ordinary-running.mid',bad=False))
    files['manifest.json']=(json.dumps(dict(provenance='Authored100ms MIDI note,5meta/sysex cancellation barriers with missing versus explicit status, ordinary running control. Separate authored native VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
