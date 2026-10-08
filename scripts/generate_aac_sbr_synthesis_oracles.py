#!/usr/bin/env python3
"""Offline direct time-index SBR synthesis references, no retained v/g arrays."""
from pathlib import Path
import argparse,re,math,struct,json,hashlib
p=argparse.ArgumentParser();p.add_argument('iso_draft_text',type=Path);a=p.parse_args()
s=a.iso_draft_text.read_text();start=s.index('Table 1.A.12 Coefficients');stop=s.index('Table 1.A.13 Noise',start)
window={int(i):float(v) for i,v in re.findall(r'(\d+)\s+(-?\d+\.\d{7,})',s[start:stop])};assert sorted(window)==list(range(640))
root=Path(__file__).resolve().parents[1];fixtures=root/'tests/fixtures/playback-errors';cases={}
for name,band,imag in [('low-real',0,False),('high-imag',63,True)]:
 slots=[[(0.0,0.0) for _ in range(64)] for _ in range(22)]
 slots[0][band]=(0.0,1.0) if imag else (1.0,0.0);cases[name]=slots
cases['dense']=[[(((t*97+k*31)%257-128)/128,((t*79+k*13)%251-125)/128) for k in range(64)] for t in range(22)]
raw=(fixtures/'aac-sbr-qmf-tones.complex-f64le').read_bytes();low=[struct.unpack('<dd',raw[i:i+16]) for i in range(0,len(raw),16)]
cases['analysis-bypass']=[low[t*32:(t+1)*32]+[(0.0,0.0)]*32 for t in range(22)]
manifest=[]
for name,slots in cases.items():
 raw=b''.join(struct.pack('<dd',re,im) for slot in slots for re,im in slot);output=[]
 # Direct contribution from each prior slot, window phase and band; no shifted
 # production history, demodulated 128-sample blocks or g extraction buffers.
 for t in range(len(slots)):
  for k in range(64):
   terms=[]
   for lag in range(min(10,t+1)):
    n=k+64*(lag%2)
    for band,(re,im) in enumerate(slots[t-lag]):
     angle=math.pi*(band+0.5)*(2*n-255)/128
     terms.append(window[64*lag+k]*(re*math.cos(angle)-im*math.sin(angle))/64)
   output.append(math.fsum(terms))
 expected=struct.pack('<'+str(len(output))+'d',*output);prefix='aac-sbr-synthesis-'+name
 (fixtures/(prefix+'.complex-f64le')).write_bytes(raw);(fixtures/(prefix+'.pcm-f64le')).write_bytes(expected)
 manifest.append(dict(name=name,slots=len(slots),input_sha256=hashlib.sha256(raw).hexdigest(),output_sha256=hashlib.sha256(expected).hexdigest()))
(fixtures/'aac-sbr-synthesis-oracles.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('4 independent 22-slot synthesis traces')
