"""Authored140ms media for rendered progress versus DSP search retirement."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-stretch-clock'
def generate():
    video,pcm=clip(1120,8000,7)
    files={'progress.mkv':video,'expected-source.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored140ms neutral VP9/8kHz mono PCM,1120 samples.64 rendered device frames at1.75rate must advance112 source frames, not a DSP search-retirement boundary. No encoder/private media/network.','frames':1120,'sample_rate':8000,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
