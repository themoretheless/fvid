#!/usr/bin/env python3
"""Authored Layer III header/side-info parser fixtures, not PCM acceptance.
Main-data bytes are markers; entropy decoding is deliberately not asserted.
"""
import struct,hashlib,json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp3-side';D.mkdir(exist_ok=True)
class Bits:
 def __init__(self):self.bits=[];self.fields={}
 def put(self,key,v,n):self.fields[key]=(len(self.bits),n);self.bits.extend((v>>i)&1 for i in range(n-1,-1,-1))
 def packed(self):
  assert len(self.bits)%8==0
  return bytes(sum(self.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(self.bits),8))
def crc(data):
 c=65535
 for byte in data:
  for i in range(7,-1,-1):
   feedback=(c>>15)^((byte>>i)&1);c=(c<<1)&65535
   if feedback:c^=0x8005
 return c
records=[];seeds={}
valid_tables=[0,1,2,3,5,6,7,8,9,10,11,12,13,15]+list(range(16,32))
for version in [3,2,0]:
 for channels in [1,2]:
  for kind,mixed in [(0,0),(1,0),(1,1),(2,0),(2,1),(3,0),(3,1)]:
   for protected in [0,1]:
    for pad in [0,1]:
     name=f'v{version}-c{channels}-w{kind}-m{mixed}-crc{protected}-p{pad}'
     mpeg1=version==3;groups=2 if mpeg1 else 1;rate=48000>>(0 if mpeg1 else 1 if version==2 else 2)
     index=9 if mpeg1 else 8;bitrate=128000 if mpeg1 else 64000;mode=3 if channels==1 else 1;extension=3
     word=(2047<<21)|(version<<19)|(1<<17)|((not protected)<<16)|(index<<12)|(1<<10)|(pad<<9)|(mode<<6)|(extension<<4)
     header=word.to_bytes(4,'big');length=(144 if mpeg1 else 72)*bitrate//rate+pad
     b=Bits();back=0 if kind==0 else 8;b.put('back',back,9 if mpeg1 else 8)
     private=5 if mpeg1 and channels==1 else 3 if mpeg1 else 1 if channels==1 else 2;b.put('private',(1<<private)-1,private)
     selections=[[((ch+i+pad)%2) for i in range(4)]if mpeg1 and ch<channels else[0]*4 for ch in range(2)]
     if mpeg1:
      for ch in range(channels):
       for i,v in enumerate(selections[ch]):b.put(f'scfsi-{ch}-{i}',v,1)
     expected=[version,rate,channels,length,protected,0,mode,extension,back]+sum(selections,[])+[groups]
     for g in range(groups):
      for ch in range(channels):
       prefix=f'{g}-{ch}-';part=33+g*17+ch*11;big=[0,1,17,288][(g+ch+kind)%4];gain=173+g*21+ch*7;compress=(11+g+ch)%16 if mpeg1 else 321+ch*31
       for key,v,n in [('part',part,12),('big',big,9),('gain',gain,8),('compress',compress,4 if mpeg1 else 9),('switch',int(kind!=0),1)]:b.put(prefix+key,v,n)
       tables=[valid_tables[(g*7+ch*13+i+kind)%len(valid_tables)]for i in range(3)];gains=[(i+ch+g+1)%8 for i in range(3)];regions=[6+g,5+ch]
       if kind:
        b.put(prefix+'block',kind,2);b.put(prefix+'mixed',mixed,1)
        for i in range(2):b.put(prefix+f'table-{i}',tables[i],5)
        tables[2]=0
        for i in range(3):b.put(prefix+f'subgain-{i}',gains[i],3)
        regions=[255,255]
       else:
        for i in range(3):b.put(prefix+f'table-{i}',tables[i],5)
        b.put(prefix+'r0',regions[0],4);b.put(prefix+'r1',regions[1],3);gains=[0,0,0]
       pre=(g+ch+pad)%2 if mpeg1 else 255
       if mpeg1:b.put(prefix+'pre',pre,1)
       scale=(g+ch+kind)%2;count=(g+ch+kind+1)%2;b.put(prefix+'scale',scale,1);b.put(prefix+'count',count,1)
       expected += [part,big,gain,compress,kind,mixed]+tables+gains+regions+[pre,scale,count]
     side=b.packed();assert len(side)==(17 if channels==1 else 32)if mpeg1 else len(side)==(9 if channels==1 else 17)
     prefix=header+(struct.pack('>H',crc(header[2:]+side))if protected else b'')+side
     payload=bytes((i*73+version*11+kind*7+channels)%256 for i in range(length-len(prefix)))
     output=prefix+payload;(D/f'{name}.frame').write_bytes(output);(D/f'{name}.expected').write_text(' '.join(map(str,expected))+'\n')
     records.append(dict(name=name,sha256=hashlib.sha256(output).hexdigest()))
     if version==3 and channels==2 and protected and not pad and not mixed and kind in [0,2]:seeds[kind]=(header,b,side,payload)
invalid=[]
for name,kind,changes in [('big-values',0,{'0-0-big':289}),('huffman-table',0,{'0-0-table-0':4}),('region-counts',0,{'0-0-r0':15,'0-0-r1':7}),('switched-normal',2,{'0-0-block':0})]:
 header,b,side,payload=seeds[kind];bits=b.bits[:]
 for key,value in changes.items():
  at,width=b.fields[key];bits[at:at+width]=[(value>>i)&1 for i in range(width-1,-1,-1)]
 data=bytes(sum(bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(bits),8))
 output=header+struct.pack('>H',crc(header[2:]+data))+data+payload
 (D/f'{name}.invalid').write_bytes(output);invalid.append(dict(file=f'{name}.invalid',sha256=hashlib.sha256(output).hexdigest()))
(D/'manifest.json').write_text(json.dumps(dict(scope='Header and side-info parser only; not compressed PCM acceptance',cases=records,invalid=invalid),indent=2)+'\n')
print(len(records),'MP3 side-info cases and',len(invalid),'semantic refusals')
