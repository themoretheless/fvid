#!/usr/bin/env python3
"""Own scalar raw-unit LTP channel PCM oracle; does not certify normative scale."""
import json,math,struct
from pathlib import Path
from generate_aac_ssr_fixtures import windows
root=Path(__file__).resolve().parent.parent/'tests/fixtures/playback-errors'
def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
gains=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
blob=bytearray();cases=[]
for n in (960,1024):
 kbd=windows(n,4);previous=0;overlap=[0.]*n;prior_pcm=[0]*n;current_pcm=[0]*n;current_overlap=[0]*n;frames=[]
 def weight(i,shape):return kbd[i] if shape else math.sin(math.pi*(i+.5)/(2*n))
 for frame in range(6):
  shape=frame%2;coef=frame%8;active=frame>=2;reverse=bool(frame%2)
  residual=[f32((-1)**(frame+k)*(9000000+700000*frame+210000*k)) for k in range(4)]
  spectrum=residual.copy()
  if active:
   physical=prior_pcm+current_pcm+current_overlap+[0]*n
   estimate=[x*gains[coef] for x in physical[n:3*n]]
   prediction=[sum(v*weight(i,previous if i<n else shape)*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i,v in enumerate(estimate)) for k in range(4)]
   order=list(range(4));
   if reverse:order.reverse()
   for j,i in enumerate(order):spectrum[i]=f32(residual[i]+prediction[i]+sum(a*prediction[order[j-k-1]] for k,a in enumerate([.25,-.0625]) if j>k))
  # TNS synthesis AR, operating on the low-band four samples only.
  ordered=list(range(4));
  if reverse:ordered.reverse()
  out=[]
  for i in ordered:
   value=spectrum[i]-sum(a*out[-k-1] for k,a in enumerate([.25,-.0625]) if len(out)>k);spectrum[i]=f32(value);out.append(value)
  transformed=[2/n*sum(v*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for k,v in enumerate(spectrum)) for i in range(2*n)]
  weighted=[v*weight(i,previous if i<n else shape) for i,v in enumerate(transformed)]
  raw=[overlap[i]+weighted[i] for i in range(n)];overlap=weighted[n:]
  prior_pcm=current_pcm;current_pcm=raw.copy();current_overlap=overlap.copy()
  frames.append(dict(shape=shape,coefficient=coef,active=active,reverse=reverse,residual=residual,reference_offset=len(blob)))
  blob.extend(struct.pack('<'+str(n)+'d',*(v/65536 for v in raw)));previous=shape
 cases.append(dict(n=n,frames=frames))
(root/'aac-ltp-channel-reference.f64le').write_bytes(blob)
(root/'aac-ltp-channel.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
print('generated 12 channel PCM frames in existing raw-unit convention')
