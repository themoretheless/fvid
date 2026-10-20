"""Author100ms note timelines whose fractional tempo segments must retain phase."""
import json,struct,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-tempo-remainder'
def tempo(delta,value):return bytes([delta,255,81,3])+value.to_bytes(3,'big')
def generate():
    start=tempo(0,100000)+b'\0\x90\x45\x64';end=b'\0\xff\x2f\0'
    tracks={'control':start+b'\x03\x80\x45\0'+end,'duplicate-tempo':start+tempo(1,100000)+tempo(1,100000)+b'\x01\x80\x45\0'+end,'changing-tempo':start+tempo(1,100001)+tempo(1,99999)+b'\x01\x80\x45\0'+end,'final-tempo':start+tempo(1,100000)+tempo(1,100000)+tempo(1,100000)+b'\0\x80\x45\0'+end}
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,track in tracks.items():files[name+'.mid']=midi(track,3);cases.append(dict(name=name,file=name+'.mid'))
    files['manifest.json']=(json.dumps(dict(provenance='Authored100ms MIDI note on/off, PPQ3, redundant or changing tempos with exact100000us cumulative duration. Short native VP9/PCM video control. No private input/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
