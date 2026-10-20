"""Author safe overflow-only MIDI decoder geometry injections and media controls."""
import json,hashlib
from pathlib import Path
from generate_midi_window_timing_regressions import generate as timing,record
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-window-geometry'
def generate():
    source=timing();files={'video.mkv':source['video.mkv'],'control.mid':source['control.mid'],'note.packet':record(1,0)}
    cases=[dict(name='normal-rate-overflow',rate=44100,channels=2,duration=(1<<64)-1),dict(name='wide-frame-count',rate=(1<<32)-1,channels=8,duration=(1<<64)-1),dict(name='sample-count-overflow',rate=1000,channels=8,duration=(1<<64)-1),dict(name='byte-count-overflow',rate=1000,channels=1,duration=(1<<64)-1),dict(name='addressable-capacity-overflow',rate=1000,channels=1,duration=1<<61)]
    files['manifest.json']=(json.dumps(dict(provenance='Five injected public MIDI decoder window geometries. Baseline debug multiplication panics before allocating; after fix each truebufferextent isunaddressable andmustrefusebefore any large allocation. Native10byte note record,authored100ms MIDI/VP9+PCM controls. No dangerous fit-sized allocation probe,encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
