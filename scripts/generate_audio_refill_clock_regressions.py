"""Authored three-packet video reproducing stale public position during pacing."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-refill-clock'
def generate():
    media,pcm=clip(video_frames=105,packets=3)
    files={'refill.mkv':media,'expected-packet.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored2.1s VP9 and three32768-sample48kHz PCM packets. No private media, encoder or network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
