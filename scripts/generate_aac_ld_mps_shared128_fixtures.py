#!/usr/bin/env python3
"""Own shared-QMF128 reproducers from the separately authored ordinary corpus."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';SOURCE=FIX/'aac-eld-mps128';OUT=FIX/'aac-ld-mps-shared128'
def main():
 OUT.mkdir(exist_ok=True);m=json.loads((SOURCE/'manifest.json').read_text());cases=[c for c in m['cases']if c['mode']=='plain'];assert len(cases)==66
 for c in cases:(OUT/c['video']).write_bytes((SOURCE/c['video']).read_bytes())
 (OUT/'core-reference.f32le').write_bytes((SOURCE/'core-reference.f32le').read_bytes());(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,provenance='Only own synthetic high-rate ELD/MPS videos and analytical core PCM from aac-eld-mps128; no private media or copied private parameters. Shared QMF API admission,not new coupled-SBR/residual qualification.'),separators=(',',':'))+'\n');print(len(cases),'own shared128 video reproducers')
if __name__=='__main__':main()
