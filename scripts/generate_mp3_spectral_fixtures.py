#!/usr/bin/env python3
"""Offline authored spectral-stage vectors, decimal oracle and inverse reorder.
Not compressed MP3 PCM qualification. Does not use external codecs or network.
"""
from pathlib import Path
from decimal import Decimal, localcontext
import json,struct,hashlib,bisect
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp3-spectral'
B=json.loads((D/'bands.json').read_text())['rates']
PRE=[0]*11+[1,1,1,1,2,2,3,3,3,2,0]
records=[]
with localcontext() as context:
 context.prec=60
 def emit(rate,kind,mixed,gain,scale,pre,sub,quant=None):
  bounds=B[str(rate)];long=bounds['long'];short=bounds['short']
  nlong=22 if kind!=2 else (8 if rate>=32000 else 6) if mixed else 0
  boundary=long[nlong]
  sf=[(i*11+gain)%16 if i<21 else 0 for i in range(22)]
  sf += [(band*7+w*3+gain)%32 if band<12 else 0 for band in range(13) for w in range(3)]
  if quant is not None:sf=[0]*61
  levels=[0,1,-1,8,-8,27,-27,64,125,-125,15,16,8206,-8206,2,-3,1000]
  q=quant if quant is not None else [levels[(i*7+gain)%len(levels)] for i in range(576)]
  expected=[]
  for output in range(576):
   if output<boundary:
    band=bisect.bisect_right(long,output)-1
    source=output;exponent=Decimal(gain-210)/4-Decimal(1+scale)/2*(sf[band]+(PRE[band] if pre else 0))
   else:
    freq,w=divmod(output,3);band=bisect.bisect_right(short,freq)-1
    width=short[band+1]-short[band]
    source=3*short[band]+w*width+freq-short[band]
    exponent=Decimal(gain-210-8*sub[w])/4-Decimal(1+scale)/2*sf[22+3*band+w]
   # Decimal exponentiation is independent of the Rust cbrt and quarter exponent.
   value=Decimal(abs(q[source]))**(Decimal(4)/3)*(Decimal(2)**exponent) if q[source] else Decimal(0)
   expected.append(float(-value if q[source]<0 else value))
  header=struct.pack('>I8B',rate,kind,mixed,gain,scale,pre,*sub)
  records.append(header+bytes(sf)+struct.pack('<576i',*q)+struct.pack('<576d',*expected))
 for rate in B:
  rate=int(rate)
  for kind,mixed in [(0,0),(1,0),(1,1),(2,0),(2,1),(3,0),(3,1)]:
   for i,gain in enumerate([0,1,209,210,254,255]):emit(rate,kind,mixed,gain,i%2,(i//2)%2,[i%8,(i+3)%8,(i+7)%8])
 for start in range(0,8207,576):
  emit(44100,0,0,210,0,0,[0,0,0],[(-1 if n%2 else 1)*n if n<=8206 else 0 for n in range(start,start+576)])
raw=b'FVMP3S1\0'+struct.pack('>I',len(records))+b''.join(records)
(D/'spectra.bin').write_bytes(raw)
(D/'manifest.json').write_text(json.dumps({'records':len(records),'sha256':hashlib.sha256(raw).hexdigest(),'scope':'Requantization and reorder stage only; no compressed PCM playback claim'},indent=2)+'\n')
print(len(records),'spectral vectors')
