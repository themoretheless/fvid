"""Authored60ms native media for backend seek/rewind flush refusal."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-flush-failure'
def generate():
    media,pcm=clip(160,8000,3,packets=3)
    files={'navigation.mkv':media,'expected-packet.f32le':pcm,'expected-before.f32le':pcm*3}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 and three20ms PCM packets. Backend flush failure is injected only after full native decoding succeeds. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
