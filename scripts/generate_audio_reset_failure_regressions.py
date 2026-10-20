"""Author60ms VP9/PCM controls for injected decoder-reset refusal."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-reset-failure'
def generate():
    video,pcm=clip(160,8000,3,packets=3)
    files={'navigation.mkv':video,'expected.f32le':pcm*3}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/PCM. The stream reset hook fails only after valid playback. This tests transition refusal, not malformed-media decoder conformance. No encoder, private media or network.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
