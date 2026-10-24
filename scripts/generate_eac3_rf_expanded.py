"""Owned expanded RF gain selection including unequal dual-mono and fallback."""
import json
from pathlib import Path
from generate_eac3_expanded_dual_drc import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-rf-expanded'
def generate():
 files=source(rf=True)
 m=json.loads(files['manifest.json']);m['accepted']=True
 files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
