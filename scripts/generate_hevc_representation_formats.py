#!/usr/bin/env python3
"""Original Annex F representation metadata continuations, no video payload."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(v):
 s=f'{v+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
m=json.loads((D/'hevc-output-sets.json').read_text());c=next(c for c in m if c['file']=='hevc-output-sets-highest.bin')
prefix=''.join(f'{v:08b}' for v in (D/c['file']).read_bytes())[:c['end']]
def fmt(chroma=1,depth=8,separate=False,width=64,height=48,crop=(1,2,3,4),present=True,window=True):
 return dict(chroma=chroma,luma_depth=depth,chroma_depth=depth,separate=separate,width=width,height=height,crop=list(crop),present=present,window=window)
cases=[]
def emit(name,formats,error=None,mapping=None,count=None):
 bits=prefix+ue(len(formats)-1 if count is None else count-1)
 previous=None;expected=[]
 for f in formats:
  bits+=f'{f["width"]:016b}{f["height"]:016b}'+str(int(f['present']))
  if f['present']:
   bits+=f'{f["chroma"]:02b}'+(str(int(f['separate'])) if f['chroma']==3 else '')+f'{f["luma_depth"]-8:04b}{f["chroma_depth"]-8:04b}'
   resolved=dict(f)
  else:
   resolved=dict(f)
   if previous:
    for key in ('chroma','luma_depth','chroma_depth','separate'):resolved[key]=previous[key]
  bits+=('1'+''.join(ue(v) for v in f['crop'])) if f['window'] else '0'
  if not f['window']:resolved['crop']=[0,0,0,0]
  sx,sy=(1,1) if resolved['separate'] else [(1,1),(2,2),(2,1),(1,1)][resolved['chroma']]
  resolved['display']=[f['width']-(resolved['crop'][0]+resolved['crop'][1])*sx,f['height']-(resolved['crop'][2]+resolved['crop'][3])*sy]
  expected.append(resolved);previous=resolved
 if len(formats)>1:
  bits+=str(int(mapping is not None))
  if mapping is not None:
   width=(len(formats)-1).bit_length()
   for v in mapping[1:]:bits+=f'{v:0{width}b}'
 bits+='11'
 file='hevc-representation-'+name+'.bin';(D/file).write_bytes(packed(bits))
 cases.append(dict(file=file,error=error,formats=expected,mapping=mapping if mapping is not None else [min(i,len(formats)-1) for i in range(4)],end=len(bits)))
for chroma,separate in [(0,False),(1,False),(2,False),(3,False),(3,True)]:
 for depth in (8,10,16):emit(f'c{chroma}-s{int(separate)}-d{depth}',[fmt(chroma,depth,separate)])
emit('inherited',[fmt(2,10),fmt(width=96,height=80,present=False),fmt(3,16,True)])
emit('mapped',[fmt(0),fmt(2,10),fmt(3,16,True)],mapping=[0,2,0,1])
mixed=fmt(3,10);mixed['chroma_depth']=16
emit('mixed-depth',[mixed]);emit('no-window',[fmt(window=False)])
emit('maximum',[fmt()]*256)
emit('missing-first',[fmt(present=False)],'lacks chroma/depth')
emit('zero',[fmt(width=0)],'zero HEVC')
emit('depth',[fmt(depth=17)],'depth exceeds')
emit('crop',[fmt(crop=(0xfffffffe,0xfffffffe,0,0))],'crop removes')
emit('bad-index',[fmt(),fmt(),fmt()],'format index out of range',mapping=[0,3,0,0])
emit('too-many',[fmt()],'out of range',count=257)
(D/'hevc-representation.json').write_text(json.dumps(cases,indent=2)+'\n')
