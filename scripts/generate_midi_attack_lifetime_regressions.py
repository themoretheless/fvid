"""Author simultaneous MIDI attack/stop events that must preserve unrelated voices."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-attack-lifetime'
def event(status,*args):return b'\0'+bytes([status,*args])
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    on=event(0x90,69,100);other=event(0x90,76,100)
    specs=[('unmatched-key',on+event(0x80,60,0),on,[69]),('other-key-release',on+other+event(0x80,69,0),other,[76]),('other-channel-release',on+event(0x87,69,0),on,[69]),('other-channel-notes-off',on+event(0xb7,123,0),on,[69]),('other-channel-sound-off',on+event(0xb7,120,0),on,[69]),('matching-release-control',on+event(0x80,69,0),b'',[]),('matching-notes-off-control',on+event(0xb0,123,0),b'',[]),('matching-sound-off-control',on+event(0xb0,120,0),b'',[])]
    for name,actual,control,keys in specs:
        suffix=b'\x60'+bytes([0x80,keys[0] if keys else 69,0])+b''.join(event(0x80,key,0) for key in keys[1:])+b'\0\xff\x2f\0'
        files[name+'.mid']=midi(actual+suffix,480);files[name+'-explicit.mid']=midi(control+suffix,480);cases.append(dict(name=name,file=name+'.mid',control=name+'-explicit.mid',audible=bool(keys)))
    files['manifest.json']=(json.dumps(dict(provenance='Eight authored100ms MIDI same-tick attack/stop sequences versus equivalent surviving-note controls. Unmatched key,other channel,one chord key release,channel-mode123/120 and three intended-silence controls. Separate authored native VP9/PCM video. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
