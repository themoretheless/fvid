#!/usr/bin/env python3
"""Authored Layer III spectra and independent timeline/convolution PCM oracle.
No compressed decoder, encoder, network, or private media is used.
"""
import math,re,struct,json,hashlib
from pathlib import Path
R=Path(__file__).resolve().parents[1]
window=[int(v)for v in re.findall(r'-?\d+',(R/'crates/fvid-codecs/src/mpeg_synthesis_window.rs').read_text().split('= [',1)[1])]
kernel=[[[window[(lag//2)*64+(lag%2)*32+j]/65536.*math.cos(math.pi*(16+j+(32 if lag%2 else 0))*(2*k+1)/64.)for k in range(32)]for j in range(32)]for lag in range(16)]
long=[[math.cos(math.pi/18*(t+.5+9)*(k+.5))for k in range(18)]for t in range(36)]
short=[[math.cos(math.pi/6*(t+.5+3)*(k+.5))for k in range(6)]for t in range(12)]
angles=[math.atan(c)for c in [-.6,-.535,-.33,-.185,-.095,-.041,-.0142,-.0037]]
def weight(kind,t):
 if kind==0:return math.sin(math.pi*(t+.5)/36)
 if kind==1:return math.sin(math.pi*(t+.5)/36)if t<18 else 1. if t<24 else math.sin(math.pi*(t-18+.5)/12)if t<30 else 0.
 if kind==3:return 0. if t<6 else math.sin(math.pi*(t-6+.5)/12)if t<12 else 1. if t<18 else math.sin(math.pi*(t+.5)/36)
 return math.sin(math.pi*(t+.5)/12)
def oracle(granules):
 # Add each transform's contribution on an absolute subband timeline, not a
 # persistent overlap buffer. Then directly convolve with the polyphase kernel.
 timeline=[[0.]*32 for _ in range(18*len(granules)+18)]
 for g,(kind,mixed,source)in enumerate(granules):
  x=source[:]
  for band in range(1,mixed if kind==2 else 32):
   for i,angle in enumerate(angles):
    a,b=x[18*band-i-1],x[18*band+i]
    x[18*band-i-1]=a*math.cos(angle)-b*math.sin(angle)
    x[18*band+i]=b*math.cos(angle)+a*math.sin(angle)
  for band in range(32):
   data=x[18*band:18*band+18]
   if not any(data):continue
   typ=0 if band<mixed else kind
   if typ!=2:
    for t in range(36):timeline[g*18+t][band]+=weight(typ,t)*sum(v*c for v,c in zip(data,long[t]))
   else:
    for w in range(3):
     for t in range(12):timeline[g*18+6+6*w+t][band]+=weight(2,t)*sum(data[3*k+w]*short[t][k]for k in range(6))
 for t,row in enumerate(timeline):
  if t%2:
   for band in range(1,32,2):row[band]=-row[band]
 sparse=[[(k,v)for k,v in enumerate(row)if v]for row in timeline]
 return [sum(kernel[lag][j][k]*v for lag in range(min(16,t+1))for k,v in sparse[t-lag])for t in range(18*len(granules))for j in range(32)]
