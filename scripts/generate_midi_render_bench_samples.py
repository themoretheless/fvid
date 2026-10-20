"""Author short variable-voice MIDI render benchmark inputs, offline."""
import json,hashlib
from pathlib import Path
from generate_midi_bend_range_regressions import cc,track
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-render-bench'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for voices in [1,8,32]:
        events=[]
        for ch in range(min(voices,16)):
            events += [(0,cc(ch,7,64+(ch%4)*16)),(0,cc(ch,11,96)),(0,cc(ch,10,ch*7)),(96,cc(ch,7,112-ch*2)),(192,cc(ch,11,127-ch)),(192,cc(ch,10,127-ch*7))]
        for i in range(voices):
            ch=i%16;key=40+i;events += [(0,bytes([0x90+ch,key,64])),(288,bytes([0x80+ch,key,0]))]
        name=f'voices-{voices}.mid';files[name]=track(events);cases.append(dict(file=name,voices=voices))
    files['manifest.json']=(json.dumps(dict(provenance='Three authored300ms MIDIperformances with1/8/32voices,channelgain/expression/pan changes andnormalrelease tails. Benchmarkinputs,notnewmediafailureclaims. NativeVP9/PCMcompanion,no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
