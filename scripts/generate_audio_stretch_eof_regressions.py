"""Authored short finite audio, shorter than a WSOLA processing segment."""
import hashlib,json
from pathlib import Path
from generate_matroska_pcm_clock_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-stretch-eof'
def generate():
    video,pcm=clip()
    files={'short-finite.mkv':video,'expected-source.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/8kHz mono PCM,480 audio frames, shorter than512-frame stretch segment. No encoder/private media/network.','rates':[500,875,1000,1250,1750,2000],'source_frames':480,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
