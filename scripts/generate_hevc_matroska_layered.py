"""Owned multilayer HEVC Matroska wrappers; no codec processes or private media."""
import json,struct
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
 rows += [dict(file='hevc-layered-vps-epoch-'+v+'.mp4')for v in ['eos','eob','split','before-first','multiple','short','bla','graph','ids','ids-split','ids-unused','idr','idr-graph','cross-bla','cross-bla-graph','mixed-reset-active','mixed-cross-reset','mixed-cross-reset-graph','mixed-cross-reset-discard','deferred-vps','deferred-vps-first']]
 rows += [dict(file='hevc-layered-vps-epoch-'+v+'.mp4',error=e)for v,e in [('illegal','HEVC active VPS changed before sequence end'),('truncated','truncated or oversized bit field'),('graph-truncated','truncated or oversized bit field')]]
 for c in rows:
  name=Path(c['file']).stem
  private,groups,size=inventory(ROOT/c['file'])
  packets=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups]
  for irregular in ([False,True]if '-vps-epoch-'in name and c.get('error') is None else[False]):
   key=name+('-irregular'if irregular else'')+'.mkv'
   pts=[17000003+i*37000001+i*i*1000003 for i in range(len(groups))]if irregular else[i*40000000 for i in range(len(groups))]
   end=pts[-1]+53000007 if irregular else len(groups)*40000000
   track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+(b''if irregular else uint(0x23e383,40000000))+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
   durations=[pts[i+1]-t if i+1<len(pts)else end-t for i,t in enumerate(pts)]
   clusters=[]
   for i,p in enumerate(packets):
    invisible=not any(((n[0]>>1)&63)<32 for n in groups[i])
    if irregular:
     block=b'\x81\0\0'+bytes([8 if invisible else 0])+p
     reference=element(0xfb,struct.pack('>q',pts[i-1]-pts[i]))if i and any(((n[0]>>1)&63)<16 for n in groups[i])else b''
     entry=element(0xa0,element(0xa1,block)+uint(0x9b,durations[i])+reference)
    else:entry=element(0xa3,b'\x81\0\0'+bytes([(0x80 if i==0 else 0)|(8 if invisible else 0)])+p)
    clusters.append(element(0x1f43b675,uint(0xe7,pts[i])+entry))
   payload=b''.join(clusters)
   info=uint(0x2ad7b1,1)+(element(0x4489,struct.pack('>d',float(end)))if irregular else b'')
   files[key]=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,info)+element(0x1654ae6b,element(0xae,track))+payload)
   cases.append(dict(file=key,control=c['file'],frames=len(groups),pts_ns=pts,durations_ns=durations,end_ns=end,irregular=irregular,error=c.get("error")))
 files['manifest.json']=(json.dumps(dict(cases=[c for c in cases if c["error"] is None],refusals=[c for c in cases if c["error"] is not None],provenance='Repository-owned layered HEVC entropy and neutral pixels, rewrapped by own EBML authoring; no private input, codec process, FFmpeg or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
