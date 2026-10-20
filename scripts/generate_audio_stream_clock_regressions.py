"""Authored60ms media for stream clock precision regressions."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-stream-clock'
def generate():
    video,pcm=clip(480,8000,3,origin=9_000_000_000_001)
    files={'clock.mkv':video,'expected-source.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 and480 PCM samples for stream timestamp precision. Stored cluster origin is9000000000001ms; decoded payload remains60ms. No encoder/private media/network.','frames':480,'sample_rate':8000,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
