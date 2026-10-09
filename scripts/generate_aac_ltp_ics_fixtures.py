#!/usr/bin/env python3
"""Own ordinary LTP ICS/pair bit writer, no decoder/FFmpeg or private media."""
import json
from pathlib import Path
root=Path(__file__).resolve().parent.parent/'tests/fixtures/playback-errors'
def field(v,w):return format(v,'0%db'%w)
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
blob=bytearray();cases=[]
for n in (960,1024):
 for seq in (0,1,3):
  for shape in (0,1):
   for bands in (0,1,40,63):
    for pair in (False,True):
     for mode in range(5 if pair else 3):
      present=mode!=0
      enabled=[mode in (2,4),pair and mode in (3,4)]
      body='0'+field(seq,2)+str(shape)+field(bands,6)+str(int(present))
      predictors=[]
      if present:
       for ch in range(2 if pair else 1):
        body+=str(int(enabled[ch]))
        if enabled[ch]:
         lag=n-ch;coef=(shape+seq+ch)%8;used=[(b+ch)%3!=0 for b in range(min(bands,40))]
         body+=field(lag,11)+field(coef,3)+''.join(str(int(x)) for x in used)
         predictors.append(dict(lag=lag,coefficient=coef,used=used))
        else:predictors.append(None)
      else:predictors=[None]*(2 if pair else 1)
      predictors+= [None]*(2-len(predictors))
      raw='101'+body+'101011';data=packed(raw)
      cases.append(dict(offset=len(blob),bytes=len(data),n=n,sequence=seq,shape=shape,bands=bands,pair=pair,predictors=predictors,groups=[1],end=3+len(body)));blob.extend(data)
 for pair in (False,True):
  for mask in range(128):
   groups=[1]
   for b in range(7):
    if mask & (1<<(6-b)):groups[-1]+=1
    else:groups.append(1)
   body='0101'+field(15,4)+field(mask,7)
   raw='101'+body+'101011';data=packed(raw)
   cases.append(dict(offset=len(blob),bytes=len(data),n=n,sequence=2,shape=1,bands=15,pair=pair,predictors=[None,None],groups=groups,end=3+len(body)));blob.extend(data)
(root/'aac-ltp-ics.bin').write_bytes(blob)
(root/'aac-ltp-ics.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
print('generated',len(cases),'LTP ICS cases')
