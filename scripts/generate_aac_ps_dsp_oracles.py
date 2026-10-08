#!/usr/bin/env python3
"""Independent full PS QMF->PCM references with six-slot lookahead.
Explicit offline generation; Decimal hybrid/convolution/transients/mixing,
then direct QMF synthesis contributions. No Rust or foreign decoder output.
"""
import json,struct,re,math
from decimal import Decimal as D, localcontext
from generate_aac_ps_decorrelation_oracles import ROOT,DEST,MAP,ZERO,mul,add,scale,response
from generate_aac_ps_hybrid_oracles import KERNELS,NAMES,TOPOLOGY,signal
from functools import lru_cache

def read(desc,filename):
 off,count=desc;raw=(DEST/filename).read_bytes()
 return [D.from_float(v[0]) for v in struct.iter_unpack('<d',raw[off:off+8*count])]
@lru_cache(None)
def synthesis_matrix(bands,odd):
 return [[complex(math.cos(math.pi*(b+.5)*(2*(k+bands*odd)-(255 if bands==64 else 127.5))/(2*bands)),
                  math.sin(math.pi*(b+.5)*(2*(k+bands*odd)-(255 if bands==64 else 127.5))/(2*bands)))/64
          for b in range(bands)] for k in range(bands)]
def synthesize(qmf,bands,window):
 # Each previous slot's independently projected contribution is weighted
 # directly by its lag. No production delay/window-shuffle history is used.
 projections=[]
 for row in qmf:
  values=[complex(float(r),float(i)) for r,i in row[:bands]]
  projections.append([[math.fsum((x*m).real for x,m in zip(values,terms)) for terms in synthesis_matrix(bands,odd)] for odd in [0,1]])
 result=[]
 for t in range(len(qmf)):
  for k in range(bands):
   result.append(math.fsum(window[(64//bands)*(bands*lag+k)]*projections[t-lag][lag%2][k] for lag in range(min(10,t+1))))
 return result

def main():
 blob=bytearray()
 def save(values):
  values=list(values);offset=len(blob);blob.extend(struct.pack('<'+'d'*len(values),*[float(v) for v in values]));return [offset,len(values)]
 def pairs(rows):return save(v for row in rows for pair in row for v in pair)
 source=(ROOT/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').read_text()
 window=[float(v) for v in re.findall(r'-?\d+\.\d+',source.split('= [',1)[1])];assert len(window)==640
 matrices=json.loads((DEST/'aac-ps-matrix-controller-oracles.json').read_text())
 sources=[(dict(kind='video',name=v['video']['file']),v['expected']) for v in matrices['videos']]
 sources += [(dict(kind='sbr-video',name=v['video']['file']),v['expected']) for v in matrices['videos']]
 sources += [(dict(kind='sequence',name=matrices['sequences'][i]['name']),matrices['sequences'][i]['frames'][:3]) for i in [0,6]]
 sources += [(dict(kind='sbr-video-30',name=v['video']['file']),v['expected']) for v in json.loads((DEST/'aac-sbr-ps-30-oracles.json').read_text())['videos']]
 cases=[]
 for source,expected in sources:
  for eof in [False,True]:
   total=sum(e['slots'] for e in expected)
   if source['kind'].startswith('sbr-video'):
    # Authored SBR payload: silent core, EOrig=128, QOrig=.5, no attack,
    # limiter density=2/gains=2, smoothing disabled (smoothing_mode=1).
    raw_noise=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[tuple(D.from_float(v) for v in struct.unpack_from('<dd',raw_noise,i*16)) for i in range(512)]
    with localcontext() as context:
     context.prec=80
     amplitude=(D(128)*D('.5')/D('1.5')).sqrt()*min(D(3).sqrt(),D('1.584893192'))
    qmf=[[scale(noise[(n*17+k-10+1)%512],amplitude) if 10<=k<27 and (n<total or not eof) else ZERO for k in range(64)] for n in range(total+6)]
   else:
    qmf=[[signal(n,k,'complex') if n<total or not eof else ZERO for k in range(64)] for n in range(total+6)]
   outputs=[[],[]];frames=[];history=[];power=[];peak=[];previous=None
   start=0
   for fi,e in enumerate(expected):
    bands=e['bands'];slots=e['slots'];bindings=MAP['hybrid'+str(bands)]
    mono=[]
    for n in range(start,start+slots):
     physical=n+6;raw=[]
     for p,name in enumerate(NAMES[bands]):
      raw.append([tuple(sum((mul(tap,qmf[physical-lag][p])[c] for lag,tap in enumerate(taps) if physical>=lag),D(0)) for c in range(2)) for taps in KERNELS[name]])
     row=[tuple(sum((raw[p][q][c] for q in indices),D(0)) for c in range(2)) for p,*indices in TOPOLOGY[bands]]
     row.extend(qmf[n][k] for k in range(len(NAMES[bands]),64));mono.append(row)
    if bands!=previous:history=[];power=[];peak=[]
    history.extend(mono);stereo=[[],[]]
    for offset,row in enumerate(mono):
     n=len(history)-slots+offset;p=[D(0)]*bands
     for value,b in zip(row,bindings):p[b['parameter']]+=value[0]**2+value[1]**2
     power.append(p)
     pe=[max((D('.76592833836465')**(n-j))*v[b] for j,v in enumerate(power)) for b in range(bands)];peak.append(pe)
     sm=[D('.25')*sum(D('.75')**(n-j)*v[b] for j,v in enumerate(power)) for b in range(bands)]
     df=[D('.25')*sum(D('.75')**(n-j)*(peak[j][b]-power[j][b]) for j in range(n+1)) for b in range(bands)]
     gain=[sm[b]/(D('1.5')*df[b]) if D('1.5')*df[b]>sm[b] else D(1) for b in range(bands)]
     diffuse=[]
     for k,b in enumerate(bindings):
      h=response(bands,k,len(history));y=ZERO
      for j in range(n+1):y=add(y,mul(h[n-j],history[j][k]))
      diffuse.append(scale(y,gain[b['parameter']]))
     coeff=read(e['coefficients'][offset],e.get('coefficients_file','aac-ps-matrix-controller-coefficients.bin'))
     left=[ZERO]*64;right=[ZERO]*64
     for k,b in enumerate(bindings):
      base=b['parameter']*8;h=[tuple(coeff[base+m*2:base+m*2+2]) for m in range(4)]
      if b['conjugate']:h=[(r,-i) for r,i in h]
      left[b['qmf']]=add(left[b['qmf']],add(mul(h[0],row[k]),mul(h[2],diffuse[k])))
      right[b['qmf']]=add(right[b['qmf']],add(mul(h[1],row[k]),mul(h[3],diffuse[k])))
     stereo[0].append(left);stereo[1].append(right)
    outputs[0].extend(stereo[0]);outputs[1].extend(stereo[1]);previous=bands
    frames.append(dict(bands=bands,slots=slots,input_start=start,qmf=[pairs(c) for c in stereo]));start+=slots
   for mode,b in [('Double',64),('Core',32)]:
    pcm=[synthesize(c,b,window) for c in outputs]
    for fi,f in enumerate(frames):f[mode]=[save(c[f['input_start']*b:(f['input_start']+f['slots'])*b]) for c in pcm]
   cases.append(dict(source=source,zero_eof=eof,input=pairs(qmf),frames=frames))
 (DEST/'aac-ps-dsp-oracles.json').write_text(json.dumps(dict(source='Own Decimal aligned hybrid/allpass/transient/matrix reference; normative direct synthesis convolution; independent numeric protocol assets',lookahead=6,cases=cases),indent=2)+'\n')
 (DEST/'aac-ps-dsp-reference.bin').write_bytes(blob)
if __name__=='__main__':main()
