#!/usr/bin/env python3
"""Equivalent authored PCM presentation gaps with contiguous decode time."""
from pathlib import Path
import json,hashlib
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-audio-middle-gap'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-audio-composition-gap'
def main():
 control=(BASE/'pcm-v0-none.mp4').read_bytes();out=b'';packet=0
 for k,b in atoms(control):
  if k==b'moof':
   boxes=dict(atoms(b));traf=dict(atoms(boxes[b'traf']))
   if b'trun' not in traf:continue
   shift=0 if packet==0 else 160
   traf[b'tfdt']=word(0,packet*160)
   trun=traf[b'trun'];assert trun==word(1,1,100)
   traf[b'trun']=word(0x801,1,104,shift)
   b=b''.join(atom(t,atom(b'tfhd',traf[b'tfhd'])+atom(b'tfdt',traf[b'tfdt'])+atom(b'trun',traf[b'trun'])) if t==b'traf' else atom(t,v) for t,v in atoms(b));packet+=1
  out+=atom(k,b)
 assert packet==3
 pcm=(BASE/'pcm-oracle.f32le').read_bytes();assert len(pcm)==480*4
 expected=pcm[:640]+bytes(640)+pcm[640:]
 files={'decode-gap.mp4':control,'composition-gap.mp4':out,'expected.f32le':expected}
 for i in range(3):files[f'pcm-{i}.packet']=(BASE/f'pcm-{i}.packet').read_bytes()
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Existing authored integer PCM and neutral VP9. Three PCM packets retain presentation positions0,320,480, durations160 at8kHz. Composition companion removes empty traf, uses contiguous decode0,160,320 and trun CTS0,160,160; data offsets adjusted for new field. No private data/parameters, encoder or network.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
