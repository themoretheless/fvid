"""Authored video/audio controls for terminal device command refusals."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-control-failure'
def generate():
    media,pcm=clip(160,8000,3,packets=3)
    files={'control.mkv':media,'expected.f32le':pcm*3}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/PCM; device resume, gain and rate failures injected after exact playback. No private media, encoder or network.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
