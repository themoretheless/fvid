#!/usr/bin/env python3
"""Authored FLAC frames + PCM oracles, per RFC9639; no external encoder."""
import struct,json,hashlib
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-native'
D.mkdir(exist_ok=True)
class Bits:
 def __init__(self):self.bits=[]
 def put(self,v,n):self.bits.extend((v>>i)&1 for i in range(n-1,-1,-1))
 def unary(self,n):self.bits.extend([0]*n+[1])
 def pack(self):
  self.bits.extend([0]*(-len(self.bits)%8));return bytes(sum(self.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(self.bits),8))
def crc(data,width,poly):
 c=0
 for v in data:
  c^=v<<(width-8)
  for _ in range(8):c=((c<<1)^(poly if c&(1<<(width-1))else 0))&((1<<width)-1)
 return c
items=[]
for depth in [4,8,12,16,20,24,32]:
 for assignment in [0,1,7,8,9,10]:
  channels=assignment+1 if assignment<8 else 2
  for kind in ['constant','verbatim','fixed0','fixed1','fixed2','fixed3','fixed4','lpc1','lpc8','lpc32']:
   for method in [0,1,2]:
    if kind in ['constant','verbatim'] and method:continue
    wasted=2 if depth>=8 else 0;limit=(1<<(depth-wasted-1))-1
    pcm=[[((i*3+c*7)%max(1,min(limit,101))-min(limit,101)//2)*(1<<wasted)for i in range(64)]for c in range(channels)]
    if kind=='constant':pcm=[[(-1 if c%2 else 1)*(1<<wasted)]*64 for c in range(channels)]
    if kind=='verbatim':
     for c in range(channels):pcm[c][0]=-(1<<(depth-1));pcm[c][1]=(1<<(depth-1))-(1<<wasted)
    planes=[v[:]for v in pcm];widths=[depth]*channels
    if assignment==8:planes=[pcm[0], [a-b for a,b in zip(*pcm)]];widths=[depth,depth+1]
    if assignment==9:planes=[[a-b for a,b in zip(*pcm)],pcm[1]];widths=[depth+1,depth]
    if assignment==10:planes=[[(a+b)//2 for a,b in zip(*pcm)],[a-b for a,b in zip(*pcm)]];widths=[depth,depth+1]
    header=bytes([255,248,0x64,(assignment<<4)])+bytes([0,63]);header+=bytes([crc(header,8,7)])
    b=Bits()
    for channel,(values,width) in enumerate(zip(planes,widths)):
     dropped=wasted-(1 if assignment==10 and channel==0 and wasted else 0)
     values=[v>>dropped for v in values];width-=dropped
     order=int(kind[5:])if kind.startswith('fixed')else int(kind[3:])if kind.startswith('lpc')else 0
     code=0 if kind=='constant'else 1 if kind=='verbatim'else 8+order if kind.startswith('fixed')else 31+order
     b.put(0,1);b.put(code,6);b.put(bool(dropped),1)
     if dropped:b.unary(dropped-1)
     if kind=='constant':b.put(values[0],width);continue
     if kind=='verbatim':
      for v in values:b.put(v,width)
      continue
     for v in values[:order]:b.put(v,width)
     if kind.startswith('lpc'):
      b.put(1,4);b.put(0,5)
      for j in range(order):b.put(1 if j==0 else 0,2)
     coefs={0:[],1:[1],2:[2,-1],3:[3,-3,1],4:[4,-6,4,-1]}
     residual=[]
     for i in range(order,64):
      predicted=values[i-1] if kind.startswith('lpc')else sum(v*values[i-j-1]for j,v in enumerate(coefs[order]))
      residual.append(values[i]-predicted)
     partition_order=0 if order>=32 else 1;size=64>>partition_order
     b.put(1 if method==1 else 0,2);b.put(partition_order,4);at=0
     for part in range(1<<partition_order):
      count=size-(order if part==0 else 0);run=residual[at:at+count];at+=count
      pbits=5 if method==1 else 4
      if method==2:
       b.put((1<<pbits)-1,pbits);raw=max((abs(v).bit_length()+1 for v in run),default=0);b.put(raw,5)
       for v in run:b.put(v,raw)
      else:
       k=3;b.put(k,pbits)
       for v in run:
        folded=2*v if v>=0 else -2*v-1;b.unary(folded>>k);b.put(folded&((1<<k)-1),k)
    frame=header+b.pack();frame+=struct.pack('>H',crc(frame,16,0x8005))
    name=f'{depth}-{assignment}-{kind}-{method}';case=struct.pack('>IBBI',8000,channels,depth,64)+frame
    (D/f'{name}.case').write_bytes(case)
    streaminfo=struct.pack('>HH',64,64)+len(frame).to_bytes(3,'big')*2+struct.pack('>Q',(8000<<44)|((channels-1)<<41)|((depth-1)<<36)|64)+bytes(16)
    (D/f'{name}.flac').write_bytes(b'fLaC'+bytes([128,0,0,34])+streaminfo+frame)
    (D/f'{name}.s32le').write_bytes(b''.join(struct.pack('<i',pcm[c][i])for i in range(64)for c in range(channels)))
    items.append(dict(name=name,sha256=hashlib.sha256(case).hexdigest()))
# Independent extended LPC cases: signed 15-bit coefficient extremes, arithmetic
# right shift, every channel count, and Rice parameters beyond the old matrix.
def save_extended(name,channels,b,pcm):
 header=bytes([255,248,0x64,(channels-1)<<4,0,63]);header+=bytes([crc(header,8,7)])
 frame=header+b.pack();frame+=struct.pack('>H',crc(frame,16,0x8005))
 case=struct.pack('>IBBI',8000,channels,32,64)+frame
 (D/f'{name}.case').write_bytes(case)
 streaminfo=struct.pack('>HH',64,64)+len(frame).to_bytes(3,'big')*2+struct.pack('>Q',(8000<<44)|((channels-1)<<41)|(31<<36)|64)+bytes(16)
 (D/f'{name}.flac').write_bytes(b'fLaC'+bytes([128,0,0,34])+streaminfo+frame)
 (D/f'{name}.s32le').write_bytes(b''.join(struct.pack('<i',pcm[c][i])for i in range(64)for c in range(channels)))
 items.append(dict(name=name,sha256=hashlib.sha256(case).hexdigest()))
for channels in range(1,9):
 for order in [1,2,7,16,32]:
  for shift in [0,3,15]:
   for k in [0,15,30]:
    b=Bits();pcm=[[((i*7+c*3)%63)-31 for i in range(64)]for c in range(channels)]
    precision=2 if k==0 else 15
    coefs=[(1 if j%2==0 else -1) if k==0 else (16383 if j%2==0 else -16384)for j in range(order)]
    for values in pcm:
     b.put(0,1);b.put(31+order,6);b.put(0,1)
     for v in values[:order]:b.put(v,32)
     b.put(precision-1,4);b.put(shift,5)
     for coef in coefs:b.put(coef,precision)
     b.put(1,2);b.put(0,4);b.put(k,5)
     for i in range(order,64):
      prediction=sum(coef*values[i-j-1]for j,coef in enumerate(coefs))//(1<<shift)
      residual=values[i]-prediction
      assert -(1<<31)<residual<(1<<31)
      folded=2*residual if residual>=0 else -2*residual-1
      b.unary(folded>>k);b.put(folded&((1<<k)-1),k)
    save_extended(f'extended-lpc-{channels}-{order}-{shift}-{k}',channels,b,pcm)
 b=Bits()
 for _ in range(channels):
  b.put(0,1);b.put(8,6);b.put(0,1);b.put(1,2);b.put(6,4)
  for _ in range(64):b.put(31,5);b.put(0,5)
 save_extended(f'extended-zero-escape-{channels}',channels,b,[[0]*64 for _ in range(channels)])
# Malformed syntax with valid frame/header CRC, to reach semantic checks.
invalid_cases=[]
for kind in ['subframe-padding','subframe-type','wasted-width','lpc-precision','lpc-shift','residual-method','partition-geometry','footer-padding','trailing-payload']:
 b=Bits()
 if kind=='subframe-padding':b.put(128,8)
 elif kind=='subframe-type':b.put(4,8)
 elif kind=='wasted-width':b.put(1,8);b.unary(31)
 elif kind in ['lpc-precision','lpc-shift']:
  b.put(64,8);b.put(0,32);b.put(15 if kind=='lpc-precision'else 1,4)
  if kind=='lpc-shift':b.put(31,5)
 else:
  b.put(16,8);b.put(2 if kind=='residual-method'else 0,2);b.put(7 if kind=='partition-geometry'else 0,4)
  if kind not in ['residual-method','partition-geometry']:
   b.put(0,4)
   for _ in range(64):b.put(1,1)
   if kind=='footer-padding':b.put(1,1)
   if kind=='trailing-payload':b.pack();b.put(0,8)
 header=bytes([255,248,0x64,0,0,63]);header+=bytes([crc(header,8,7)])
 frame=header+b.pack();frame+=struct.pack('>H',crc(frame,16,0x8005))
 (D/f'{kind}.invalid').write_bytes(frame)
 invalid_cases.append(dict(file=f'{kind}.invalid',sha256=hashlib.sha256(frame).hexdigest()))
(D/'manifest.json').write_text(json.dumps(dict(reference='https://www.rfc-editor.org/rfc/rfc9639.html',cases=items,invalid_cases=invalid_cases,video_companion='../g711-alaw-2ch.avi'),indent=2)+'\n')
print(len(items),'authored FLAC cases')
