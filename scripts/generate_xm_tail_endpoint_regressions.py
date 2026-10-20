"""Author fractional-tempo XM release-tail endpoint rounding probes."""
import json,hashlib,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/xm-tail-endpoint'
def generate():
    original=(ROOT/'tests/fixtures/xm/two-notes.xm').read_bytes();files={'video.mkv':pcm()['video.mkv']};cases=[]
    for bpm in [32,63,125,127,199,255]:
        data=bytearray(original);struct.pack_into('<H',data,78,bpm);name=f'tempo-{bpm}.xm';files[name]=bytes(data);cases.append(dict(file=name,bpm=bpm))
    files['manifest.json']=(json.dumps(dict(provenance='Six authored three-row XM files with tempos32/63/125/127/199/255andexplicit keyoff before tail. Test independently rounds absolute score-plus-tail endpoint,not separate start/length. Native decoded tail is entirely silent,allowing exact zero extension oracle. Short authored native VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
