"""Record authored XM references for seeks on published millisecond boundaries."""
import json,hashlib
from pathlib import Path
from generate_xm_sample_grid_regressions import generate as grid
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-published-seek'
def generate():
    authored=grid();files={'video.mkv':authored['video.mkv']}
    cases=[dict(file=n,source='../xm-sample-grid/'+n,sha256=hashlib.sha256(b).hexdigest()) for n,b in authored.items() if n.endswith('.xm')]
    files['manifest.json']=(json.dumps(dict(provenance='Authored31row held-note XM references read in place, published packet boundary seeks at fractional row tempos. Separate authored native VP9/PCM companion. No encoder/private media/network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
