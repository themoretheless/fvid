"""Authored60ms media for producer EOF versus device-consumption EOF."""
import hashlib,json
from pathlib import Path
from generate_matroska_pcm_clock_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-eof-drain'
def generate():
    video,pcm=clip()
    files={'queued-tail.mkv':video,'expected.f32le':pcm}
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 and8kHz mono PCM, three160-sample blocks. Test holds the production CPAL callback until source EOF. No encoder/private media/network.','frames':480,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
