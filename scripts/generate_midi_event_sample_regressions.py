"""Author sub-sample MIDI onset/release and window-tail timing controls."""
import json,hashlib
from pathlib import Path
from generate_midi_window_timing_regressions import record
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-event-sample'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,actual,expected in [('onset',[record(1,124),b''],[record(1,125),b'']),('release',[record(1,0)+record(2,124),b''],[record(1,0)+record(2,125),b'']),('tail-onset',[record(1,99999),b''],[b'',record(1,0)]),('tail-release',[record(1,0)+record(2,99999),b''],[record(1,0),record(2,0)])]:
        for mode,packets in [('actual',actual),('control',expected)]:
            for i,data in enumerate(packets):files[f'{name}-{mode}-{i}.packet']=data
        cases.append(dict(name=name))
    files['manifest.json']=(json.dumps(dict(provenance='Authored native MIDI200ms two-window scripts at8kHz.124us events should first affect125us sample,99999us tail events update state for100ms next window without advancing a prior sample. Onset/release boundary controls and short native VP9/PCM video. No encoder/private media/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
