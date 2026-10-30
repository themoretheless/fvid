"""Own three-picture witness for transient Seek interruption and recovery."""
import hashlib,json
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-interrupted-seek'
if __name__=='__main__':
    files=generate();OUT.mkdir(parents=True,exist_ok=True)
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps(dict(
        provenance='Regenerated owned VP9 colored keyframes and analytic YUV16 oracles. No input media copied, external encoder or network.',
        failure='A finite test I/O adapter returns Interrupted twice on a seek, optionally after moving the cursor. Opening, whole-packet/range reads and software playback must retry the absolute/end seek; permanent errors must still propagate.',
        timescale=50,pts=[0,1,2],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
