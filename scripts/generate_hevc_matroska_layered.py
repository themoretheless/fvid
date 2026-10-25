"""Owned multilayer HEVC Matroska wrappers; no codec processes or private media."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory
from generate_ffv1_gray_fixtures import element,uint
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-matroska-layered'
def generate():
 files={};cases=[]
 rows=json.loads((ROOT/'hevc-layered-reconstruction.json').read_text())['cases']
 rows += [dict(file='hevc-inband-layered-dispatch/'+c['name']+'.mp4')for c in json.loads((ROOT/'hevc-inband-layered-dispatch/manifest.json').read_text())]
 rows += [dict(file=c['file'],error=c['error'])for c in json.loads((ROOT/'hevc-layered-output-policy.json').read_text())['cases']]
 for c in rows:
  name=Path(c['file']).stem
  private,groups,size=inventory(ROOT/c['file'])
  packets=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups]
  track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
  payload=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+(bytes([(0x80 if i==0 else 0)|(0x08 if not any(((n[0]>>1)&63)<32 for n in groups[i])else 0)]))+p))for i,p in enumerate(packets))
  files[name+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+payload)
  cases.append(dict(file=name+'.mkv',control=c['file'],frames=len(groups),interval_ns=40000000,error=c.get("error")))
 files['manifest.json']=(json.dumps(dict(cases=[c for c in cases if c["error"] is None],refusals=[c for c in cases if c["error"] is not None],provenance='Repository-owned layered HEVC entropy and neutral pixels, rewrapped by own EBML authoring; no private input, codec process, FFmpeg or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
