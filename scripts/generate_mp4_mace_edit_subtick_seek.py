"""Authored stateful MACE edit/silence/repeat seek controls; no external tools or private media."""
from pathlib import Path
import json,hashlib
from generate_mp4_mace_transitions import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-mace-edit-subtick-seek'
def generate():
    original=source()
    names=['control-0-edits','control-1-edits','control-2-edits','control-6mono-edits','control-6mono-grouped-edits','edits','fragmented-edits','hybrid-edits']
    files={n+'.mp4':original[n+'.mp4']for n in names}
    files['manifest.json']=(json.dumps(dict(provenance='Own deterministic arithmetic MACE bytes and neutral VP9 from the standalone MACE generator. No FFmpeg,network,private media or parameters.',sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
