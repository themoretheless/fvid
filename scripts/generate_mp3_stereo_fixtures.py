#!/usr/bin/env python3
"""Authored stereo spectra with independent per-line bounds and decimal gains.
Offline stage fixtures only; not compressed MP3 PCM acceptance.
"""
from pathlib import Path
import json,struct,hashlib,math,bisect
from decimal import Decimal,localcontext
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp3-stereo';D.mkdir(exist_ok=True)
B=json.loads((D.parent/'mp3-spectral/bands.json').read_text())['rates'];records=[]
with localcontext() as ctx:
 ctx.prec=60
 for rate,bb in B.items():
  rate=int(rate);version=3 if rate>=32000 else 2 if rate>=16000 else 0
  lb,sb=bb['long'],bb['short']
  for kind,mixed in [(0,0),(1,0),(1,1),(2,0),(2,1),(3,0),(3,1)]:
   nlong=22 if kind!=2 else (8 if rate>=32000 else 6) if mixed else 0;boundary=lb[nlong]
   for pattern in range(3):
    left=[(i*19+pattern*3)%41-20 for i in range(576)];right=[0]*576
    if pattern==1:
     if kind==2:
      for w,band in enumerate([4,8,11]):right[3*sb[band]+w]=w+1
     else:right[lb[9]+1]=3;right[lb[2]]=-2
    elif pattern==2:
     if nlong:right[lb[2]]=-5
     else:right[3*sb[1]]=2
    for extension in [1,3]:
     for scale in ([0]if version==3 else [0,1]):
      values=[];widths=[]
      for i in range(61):
       width=3 if version==3 else (i+pattern)%6
       value=(i*7+pattern*5)%(1<<width) if width else 0
       values.append(value);widths.append(width)
      expected=[[],[]]
      for i in range(576):
       if i<boundary:
        band=bisect.bisect_right(lb,i)-1;code=min(band,20)
        eligible=not any(right[j] for j in range(lb[band],576))
       else:
        freq,w=divmod(i,3);band=bisect.bisect_right(sb,freq)-1;code=22+3*min(band,11)+w
        eligible=not any(right[3*f+w] for f in range(sb[band],192))
       pos,width=values[code],widths[code]
       legal=pos<7 if version==3 else width==0 or pos!=(1<<width)-1
       if eligible and legal:
        if version==3:
         # sin/cos panning independently evaluates the specified tan ratios.
         sine,cosine=math.sin(pos*math.pi/12),math.cos(pos*math.pi/12)
         kl,kr=sine/(sine+cosine),cosine/(sine+cosine)
         if pos==6:kl,kr=1.,0.
         l,r=left[i]*kl,left[i]*kr
        else:
         a=Decimal(2)**(-Decimal((pos+1)//2)*Decimal(1+scale)/4)
         kl,kr=(Decimal(1),a)if pos%2==0 else(a,Decimal(1))
         l,r=float(Decimal(left[i])*kl),float(Decimal(left[i])*kr)
       elif extension&2:
        l=float(Decimal(left[i]+right[i])/Decimal(2).sqrt());r=float(Decimal(left[i]-right[i])/Decimal(2).sqrt())
       else:l,r=left[i],right[i]
       expected[0].append(l);expected[1].append(r)
      records.append(struct.pack('>I5B',rate,version,kind,mixed,extension,scale)+bytes(values+widths)
                     +struct.pack('<1152h',*(left+right))+struct.pack('<1152d',*(expected[0]+expected[1])))
raw=b'FVMP3T1\0'+struct.pack('>I',len(records))+b''.join(records)
(D/'stereo.bin').write_bytes(raw);(D/'manifest.json').write_text(json.dumps({'records':len(records),'sha256':hashlib.sha256(raw).hexdigest(),'scope':'Joint stereo spectral stage, not compressed PCM playback'},indent=2)+'\n')
print(len(records),'stereo vectors')
