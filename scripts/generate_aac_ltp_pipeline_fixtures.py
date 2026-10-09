#!/usr/bin/env python3
"""Own sparse-history direct cosine/FIR/interval references, no codec executables."""
import json,math,struct
from pathlib import Path
from generate_aac_ssr_fixtures import windows
root=Path(__file__).resolve().parent.parent/'tests/fixtures/playback-errors'
gains=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
raw=bytearray();gold=bytearray();cases=[]
for n in (960,1024):
 small=n//8;flat=(n-small)//2;kbd={n:windows(n,4),small:windows(small,6)}
 def weight(size,i,shape):return kbd[size][i] if shape else math.sin(math.pi*(i+.5)/(2*size))
 # Physical history: prior PCM, current PCM, current overlap, future zero.
 blocks=[]
 for phase in (0,1,2):
  block=[0.]*n
  for j,i in enumerate([0,1,n//4,n//2-1,n//2,n-2,n-1]):block[i]=(-1)**(j+phase)*(300+31*j+17*phase+.5)
  blocks.append(block)
 history=[max(-32768,min(32767,round(x))) for block in blocks for x in block]+[0]*n
 for seq in (0,1,3):
  for previous,current in ((0,0),(0,1),(1,0),(1,1)):
   lag={0:n,1:n//2,3:2*n}[seq];coef=(seq+previous+current*3)%8
   estimate=[v*gains[coef] for v in history[2*n-lag:4*n-lag]]
   sparse=[]
   for i,v in enumerate(estimate):
    if seq==1 and i>=n:
     j=i-n;w=1. if j<flat else weight(small,small+j-flat,current) if j<flat+small else 0.
    elif seq==3 and i<n:w=0. if i<flat else weight(small,i-flat,previous) if i<flat+small else 1.
    else:w=weight(n,i,previous if i<n else current)
    if v*w:sparse.append((i,v*w))
   prediction=[sum(v*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i,v in sparse) for k in range(n)]
   offsets=[0,4,16,32,n];used=[True,False,True,True]
   # Two disjoint TNS intervals, original-input direct FIR, opposite directions.
   filters=[dict(length=1,reverse=bool(current),lpc=[.25,-.0625]),dict(length=3,reverse=not bool(current),lpc=[-.125])]
   encoded=prediction.copy();top=4
   for f in filters:
    bottom=max(0,top-f['length']);indices=list(range(offsets[bottom],offsets[top]));top=bottom
    if f['reverse']:indices.reverse()
    original=[prediction[i] for i in indices]
    for j,i in enumerate(indices):encoded[i]=original[j]+sum(a*original[j-k-1] for k,a in enumerate(f['lpc']) if j>k)
   residual=[(i%19-9)*.25 for i in range(n)]
   expected=[r+(encoded[i] if any(used[b] and offsets[b]<=i<offsets[b+1] for b in range(4)) else 0.) for i,r in enumerate(residual)]
   c=dict(n=n,sequence=seq,previous=previous,current=current,lag=lag,coefficient=coef,offsets=offsets,used=used,filters=filters,input_offset=len(raw),reference_offset=len(gold));cases.append(c)
   raw.extend(struct.pack('<'+str(3*n)+'d',*(v for block in blocks for v in block)))
   gold.extend(struct.pack('<'+str(n)+'f',*expected))
(root/'aac-ltp-pipeline-input.f64le').write_bytes(raw)
(root/'aac-ltp-pipeline-reference.f32le').write_bytes(gold)
(root/'aac-ltp-pipeline.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
print('generated',len(cases),'own composite prediction references')
