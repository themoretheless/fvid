"""Authored native synth references for command service between seek-history windows."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/synth-seek-control'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for kind,path,target in [('midi','tests/fixtures/midi/two-notes.mid',1500),('xm','tests/fixtures/playback-errors/xm-sample-grid/held-125.xm',1920)]:
        cases.append(dict(kind=kind,source=path,target_ms=target,sha256=hashlib.sha256((ROOT/path).read_bytes()).hexdigest()))
    files['manifest.json']=(json.dumps(dict(provenance='Authored MIDI/XM native references read in place. Regression gates native decoder calls at known history-window boundaries to prove pause is serviced before a second decode, without host-speed timing assumptions. Separate authored native VP9/PCM video. No encoder/network/private media.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
