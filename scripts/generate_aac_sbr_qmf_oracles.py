#!/usr/bin/env python3
"""Explicit offline normative-window extraction and independent direct QMF oracle."""
from pathlib import Path
from decimal import Decimal
import argparse, re, html, math, struct, json, hashlib
p=argparse.ArgumentParser();p.add_argument('standard_html',type=Path);p.add_argument('iso_draft_text',type=Path);a=p.parse_args()
s=a.standard_html.read_text();start=s.index('id="Par46334"');stop=s.index('Таблица А.90',start);published={}
for row in re.findall('<tr[^>]*>(.*?)</tr>',s[start:stop],re.S):
 c=[re.sub(r'\s+','',html.unescape(re.sub('<[^>]+>','',x))) for x in re.findall('<td[^>]*>(.*?)</td>',row,re.S)]
 if len(c)==2 and c[0].isdigit():published[int(c[0])]=c[1].replace(',','.')
s=a.iso_draft_text.read_text();start=s.index('Table 1.A.12 Coefficients');stop=s.index('Table 1.A.13 Noise',start)
iso={int(i):v for i,v in re.findall(r'(\d+)\s+(-?\d+\.\d{7,})',s[start:stop])}
assert sorted(published)==sorted(iso)==list(range(640))
assert all(Decimal(published[i])==Decimal(iso[i]) for i in range(640))
root=Path(__file__).resolve().parents[1];fixtures=root/'tests/fixtures/playback-errors'
(root/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').write_text('// Normative QMF window: GOST R53556.4-2013 A.89 / ISO w4611 1.A.12.\n// Generated explicitly by scripts/generate_aac_sbr_qmf_oracles.py.\npub(super) const WINDOW: [f64; 640] = [\n'+''.join('    '+iso[i]+',\n' for i in range(640))+'];\n')
cases={'impulse-first':[1.0]+[0.0]*703,'impulse-boundary':[0.0]*31+[1.0,-0.5]+[0.0]*671,
 'dense':[((i*73+19)%257-128)/256 for i in range(704)],
 'tones':[0.25*math.sin(2*math.pi*3.5*i/32)+0.125*math.cos(2*math.pi*11.5*i/32) for i in range(704)]}
manifest=[]
for name,pcm in cases.items():
 raw=struct.pack('<'+str(len(pcm))+'f',*pcm);pcm=list(struct.unpack('<'+str(len(pcm))+'f',raw));outputs=[]
 # Direct time-index convolution: no mutable history or intermediate u vector.
 for last in range(31,len(pcm),32):
  for band in range(32):
   terms=[]
   for lag in range(min(320,last+1)):
    phase=math.pi*(band+0.5)*(2*(lag%64)-0.5)/64
    weighted=2*pcm[last-lag]*float(iso[2*lag])
    terms.append((weighted*math.cos(phase),weighted*math.sin(phase)))
   outputs.extend([math.fsum(t[0] for t in terms),math.fsum(t[1] for t in terms)])
 expected=struct.pack('<'+str(len(outputs))+'d',*outputs);prefix='aac-sbr-qmf-'+name
 (fixtures/(prefix+'.f32le')).write_bytes(raw);(fixtures/(prefix+'.complex-f64le')).write_bytes(expected)
 manifest.append(dict(name=name,blocks=len(pcm)//32,input_sha256=hashlib.sha256(raw).hexdigest(),output_sha256=hashlib.sha256(expected).hexdigest()))
(fixtures/'aac-sbr-qmf-oracles.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('640 verified window coefficients; 4 independent 22-block QMF traces')
