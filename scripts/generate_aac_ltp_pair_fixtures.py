#!/usr/bin/env python3
"""Own LTP CPE packets/videos; no private media, foreign decoder or FFmpeg."""
import json, math, struct
from generate_aac_ssr_fixtures import windows
from generate_aac_main_tools_fixtures import channel,ics
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
def scalar_reference(common, mode):
 n=1024; kbd=windows(n,4); gains=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
 def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
 def weight(i,shape):return kbd[i] if shape else math.sin(math.pi*(i+.5)/(2*n))
 cosine=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(8)]
 history=[[0.]*(4*n) for _ in range(2)]; overlaps=[[0.]*n for _ in range(2)]; previous=[0,0]; out=bytearray()
 for frame in range(12):
  residual=[[v*1024. for v in q] for q in ([1,-1,1,-1,0,1,-1,1],[-1,0,1,1,-1,1,0,-1])]
  if common and mode:
   for k in range(4 if mode==1 else 8):
    a,b=residual[0][k],residual[1][k];residual[0][k]=a+b;residual[1][k]=a-b
  pcm=[]
  for ch in range(2):
   shape=frame%2 if common else (frame+ch)%2
   active=frame>=3 and frame%4 in ((0,1,3) if ch==0 else (0,2,3))
   weights=[weight(i,previous[ch] if i<n else shape) for i in range(2*n)]
   spectrum=residual[ch]
   if active:
    lag=n-ch*17;gain=gains[(frame+ch*3)%8]
    estimate=[v*gain*w for v,w in zip(history[ch][2*n-lag:4*n-lag],weights)]
    spectrum=[f32(v+sum(a*b for a,b in zip(estimate,cosine[k]))) for k,v in enumerate(spectrum)]
   transformed=[2/n*sum(spectrum[k]*cosine[k][i] for k in range(8))*weights[i] for i in range(2*n)]
   raw=[overlaps[ch][i]+transformed[i] for i in range(n)];overlaps[ch]=transformed[n:]
   history[ch]=history[ch][n:2*n]+raw+overlaps[ch]+[0.]*n;previous[ch]=shape;pcm.append(raw)
  for i in range(n):out.extend(struct.pack('<ff',pcm[0][i]/65536,pcm[1][i]/65536))
 return out
blob=bytearray();cases=[]
for common,mode in [(True,0),(True,1),(True,2),(False,0)]:
 name=f'common-ms{mode}' if common else 'independent';rows=[]
 for frame in range(12):
  active=[frame>=3 and frame%4 in (0,1,3),frame>=3 and frame%4 in (0,2,3)]
  def predictor(ch):return field(1024-ch*17,11)+field((frame+ch*3)%8,3)+'11'
  def info(ch):
   base=ics(0,2,False,shape=(frame+ch)%2 if not common else frame%2)
   return base[:-1]+'11'+predictor(ch) if active[ch] else base
  if common:
   header=ics(0,2,False,shape=frame%2)
   if any(active):header=header[:-1]+'1'+''.join(str(int(active[ch]))+(predictor(ch) if active[ch] else '') for ch in (0,1))
   prefix='1'+header+field(mode,2)+('10' if mode==1 else '')
   infos=['','']
  else:prefix='0';infos=[info(0),info(1)]
  qleft=[1,-1,1,-1,0,1,-1,1];qright=[-1,0,1,1,-1,1,0,-1]
  raw=packed('0010000'+prefix+channel(0,[1,1],[qleft],info=infos[0])+channel(0,[1,1],[qright],info=infos[1])+'111')
  rows.append(dict(offset=len(blob),bytes=len(raw),active=active));blob.extend(raw)
 c=dict(name=name,asc=packed(field(4,5)+frequency(24000)+'0010'+'000').hex(),frames=rows,container_rate=24000,container_frame_samples=1024,channels=2,slots=16,bands=32,samples=12288,pcm_offset=0)
 (DEST/f'aac-ltp-pair-{name}-scalar-reference.f32le').write_bytes(scalar_reference(common,mode))
 c['video']=video_fixture([c],blob,channels=2,filename='aac-ltp-pair-'+name+'-synthetic.mp4');cases.append(c)
(DEST/'aac-ltp-pair-packets.bin').write_bytes(blob)
(DEST/'aac-ltp-pair.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
print('generated',len(cases),'LTP pair videos')
