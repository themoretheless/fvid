"""Reuse the owned, short neutral-video/late-PS witness for cold stack acceptance."""
from pathlib import Path
import json,hashlib
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ps-cold-stack'
def generate():
    source='he-aac-ps-inband-export-lc-960-synthetic.mp4'
    data=(OUT.parent/source).read_bytes()
    return {'late-ps.mp4':data,'manifest.json':(json.dumps(dict(source=source,sha256=hashlib.sha256(data).hexdigest(),frames=9600,rate=48000,channels=2,provenance='Existing FVid-owned authored PS packets and neutral video; no private media or codec parameters.'),indent=2)+'\n').encode()}
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
