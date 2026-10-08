#!/usr/bin/env python3
"""Explicit offline PS decorrelation goldens, no Rust/decoder/FFmpeg output.
Closed-form geometric allpass impulse responses + direct convolution;
transient smoothing via full weighted histories, not runtime recurrences.
"""
import json,struct
from decimal import Decimal as D
from functools import lru_cache
from pathlib import Path
from generate_aac_ps_mixing_oracles import PI,sincos
ROOT=Path(__file__).resolve().parents[1];DEST=ROOT/'tests/fixtures/playback-errors'
ZERO=(D(0),D(0))
A=list(map(D,['.65143905753106','.56471812200776','.48954165955695']))
Q=list(map(D,['.43','.75','.347']));DELAY=[3,4,5]
CENTERS={20:['-3/8','-1/8','1/8','3/8','5/8','7/8','5/4','7/4','9/4','11/4'],
34:['1/12','3/12','5/12','7/12','9/12','11/12','13/12','15/12','17/12','-5/12','-3/12','-1/12',
'17/8','19/8','5/8','7/8','9/8','11/8','13/8','15/8','9/4','11/4','13/4','7/4','17/4','11/4','13/4','15/4','17/4','19/4','21/4','15/4']}
MAP=json.loads((DEST/'aac-ps-mapping-protocol.json').read_text())
def mul(x,y):return x[0]*y[0]-x[1]*y[1],x[0]*y[1]+x[1]*y[0]
def add(x,y):return x[0]+y[0],x[1]+y[1]
def scale(x,a):return x[0]*a,x[1]*a
def phase(q,center):sn,cs=sincos(-PI*q*center);return cs,sn
def params(bands,k):
 cutoff,allpass,short=(10,30,42) if bands==20 else (32,50,62)
 if k>=allpass:return dict(delay=14 if k<short else 1)
 if k<cutoff:
  n,d=CENTERS[bands][k].split('/');center=D(n)/D(d)
 else:center=D(k)+D('.5')-(7 if bands==20 else 27)
 decay=max(D(0),1-D('.05')*max(0,k-cutoff))
 return dict(center=center,decay=decay,initial=phase(D('.39'),center),
             feedback=[a*decay for a in A],phases=[phase(q,center) for q in Q])
@lru_cache(None)
def response(bands,k,length):
 p=params(bands,k);h=[ZERO]*length
 if 'delay' in p:
  if p['delay']<length:h[p['delay']]=(D(1),D(0))
  return h
 if length>2:h[2]=p['initial']
 for a,q,d in zip(p['feedback'],p['phases'],DELAY):
  terms=[(0,(-a,D(0)))];qpower=(D(1),D(0))
  # (Q*z^-d-a)/(1-a*Q*z^-d) = -a +
  # (1-a^2)*sum_{j>=1}(a^(j-1)*Q^j*z^(-j*d)).
  for j in range(1,(length-1)//d+1):
   qpower=mul(qpower,q);terms.append((j*d,scale(qpower,(1-a*a)*a**(j-1))))
  result=[ZERO]*length
  for lag,v in enumerate(h):
   if v==ZERO:continue
   for shift,c in terms:
    if lag+shift<length:result[lag+shift]=add(result[lag+shift],mul(v,c))
  h=result
 return h

def main():
 blob=bytearray()
 def save(values):
  start=len(blob);flat=list(values);blob.extend(struct.pack('<'+'d'*len(flat),*[float(v) for v in flat]));return [start,len(flat)]
 def pairs(slots):return save(v for slot in slots for c in slot for v in c)
 def floats(desc,filename):
  data=(DEST/filename).read_bytes();off,count=desc
  return [D.from_float(v[0]) for v in struct.iter_unpack('<d',data[off:off+count*8])]
 scenarios=[('silence',[(20,64,True,64)],'silence'),
 ('twenty_impulse',[(20,128,True,64)],'impulse'),('thirty_four_impulse',[(34,128,True,64)],'impulse'),
 ('twenty_bursts',[(20,96,True,64)],'bursts'),('thirty_four_bursts',[(34,96,True,64)],'bursts'),
 ('grid32',[(20,32,True,64),(34,32,True,64),(20,32,True,64)],'complex'),
 ('grid30',[(34,30,True,64),(20,30,True,64),(34,30,True,64)],'complex'),
 ('full_reset',[(20,32,True,64),(20,32,False,64),(20,32,True,64)],'complex'),
 ('partial_twenty',[(20,32,True,64),(20,32,True,16),(20,32,True,64)],'complex'),
 ('partial_thirty_four',[(34,32,True,64),(34,32,True,16),(34,32,True,64)],'complex'),
 ('hybrid_video',[(20,32,True,64),(34,32,True,64),(20,32,True,64)],'hybrid')]
 bank=json.loads((DEST/'aac-ps-hybrid-oracles.json').read_text());bcase=next(c for c in bank['cases'] if c['name']=='grid_video')
 matrix=json.loads((DEST/'aac-ps-matrix-controller-oracles.json').read_text())
 video=next(v for v in matrix['videos'] if v['video']['file']==bcase['video'])
 coeff=[]
 for bands in [20,34]:
  for k in range(71 if bands==20 else 91):
   p=params(bands,k)
   coeff.append(dict(bands=bands,k=k,delay=p.get('delay'),
    values=None if 'delay' in p else [float(p['center']),float(p['decay']),
     *[float(v) for c in [p['initial'],*p['phases']] for v in c],*[float(v) for v in p['feedback']]]))
 cases=[]
 for name,parts,kind in scenarios:
  frames=[];history=[];starts=[];powers=[];peak=[];old=None;absolute=0
  for fi,(bands,count,present,limit) in enumerate(parts):
   bindings=MAP['hybrid'+str(bands)];width=len(bindings)
   changed=old is not None and old!=bands
   if old!=bands or not present:
    history=[];starts=[0]*width;powers=[];peak=[]
   if kind=='hybrid':
    flat=floats(bcase['frames'][fi]['output'],'aac-ps-hybrid-reference.bin')
    input=[[tuple(flat[n*width*2+k*2:n*width*2+k*2+2]) for k in range(width)] for n in range(count)]
   else:
    input=[]
    for n in range(absolute,absolute+count):
     row=[]
     for k in range(width):
      if kind=='silence':v=ZERO
      elif kind=='impulse':v=(D(int(n==0))/(k+1),D(int(n==0))/(k+2))
      else:
       amp=D(1) if kind=='complex' or n%32<8 or 17<=n%32<24 else D(0)
       v=(amp*D((13*n+7*k)%23-11)/16,amp*D((3*n+11*k)%31-15)/32)
      row.append(v)
     input.append(row)
   for k,b in enumerate(bindings):
    if b['qmf']>=limit:starts[k]=len(history)
   history.extend(input);outputs=[];raw=[];gp=[];pp=[];smooth=[];difference=[]
   for offset,row in enumerate(input):
    n=len(history)-count+offset;p=[D(0)]*bands
    for c,binding in zip(row,bindings):p[binding['parameter']]+=c[0]**2+c[1]**2
    powers.append(p)
    pe=[max((D('.76592833836465')**(n-j))*v[b] for j,v in enumerate(powers)) for b in range(bands)];peak.append(pe)
    sm=[D('.25')*sum((D('.75')**(n-j))*v[b] for j,v in enumerate(powers)) for b in range(bands)]
    df=[D('.25')*sum((D('.75')**(n-j))*(peak[j][b]-powers[j][b]) for j in range(n+1)) for b in range(bands)]
    gain=[sm[b]/(D('1.5')*df[b]) if D('1.5')*df[b]>sm[b] else D(1) for b in range(bands)]
    r=[]
    for k in range(width):
     h=response(bands,k,len(history));y=ZERO
     for j in range(starts[k],n+1):y=add(y,mul(h[n-j],history[j][k]))
     r.append(y)
    raw.append(r);outputs.append([scale(c,gain[b['parameter']]) for c,b in zip(r,bindings)])
    gp.append(gain);pp.append(p);smooth.append(sm);difference.append(df)
   f=dict(bands=bands,slots=count,previous_ps_present=present,qmf_limit=limit,
     input=pairs(input),raw=pairs(raw),output=pairs(outputs),power=save(v for row in pp for v in row),
     gains=save(v for row in gp for v in row),peak=save(peak[-1]),smooth=save(smooth[-1]),difference=save(difference[-1]))
   if kind=='hybrid':
    left=[];right=[]
    for n,(mono,diffuse) in enumerate(zip(input,outputs)):
     matrices=floats(video['expected'][fi]['coefficients'][n],'aac-ps-matrix-controller-coefficients.bin')
     l=[];r=[]
     for k,b in enumerate(bindings):
      start=b['parameter']*8;h=[tuple(matrices[start+m*2:start+m*2+2]) for m in range(4)]
      if b['conjugate']:h=[(re,-im) for re,im in h]
      l.append(add(mul(h[0],mono[k]),mul(h[2],diffuse[k])))
      r.append(add(mul(h[1],mono[k]),mul(h[3],diffuse[k])))
     left.append(l);right.append(r)
    f['stereo_hybrid']=[pairs(left),pairs(right)];inverse=[]
    for channel in [left,right]:
     qmf=[]
     for row in channel:
      values=[ZERO]*64
      for c,b in zip(row,bindings):values[b['qmf']]=add(values[b['qmf']],c)
      qmf.append(values)
     inverse.append(pairs(qmf))
    f['stereo_qmf']=inverse
   frames.append(f);old=bands;absolute+=count
  case=dict(name=name,frames=frames)
  if kind=='hybrid':case['video']=bcase['video']
  cases.append(case)
 (DEST/'aac-ps-decorrelation-oracles.json').write_text(json.dumps(dict(source='GOST R 53556.8-2013 6.4.5/6.4.6.1/A.3; own Decimal closed-form impulse response and weighted histories',coefficients=coeff,cases=cases),indent=2)+'\n')
 (DEST/'aac-ps-decorrelation-reference.bin').write_bytes(blob)
if __name__=='__main__':main()
