#!/usr/bin/env python3
"""Authored PCM/VP9 clock and seek cases; explicit offline generation."""
import json,hashlib,struct
from fractions import Fraction
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/pcm-timeline'
def u(tag,n):return ebml(tag,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def value(n,c):return ((n*(2*c+1))%127-63)/128
FORMATS=[('u8',8,8,'A_PCM/INT/LIT',False),('s16le',16,16,'A_PCM/INT/LIT',False),('s16be',16,16,'A_PCM/INT/BIG',True),('s24le',24,24,'A_PCM/INT/LIT',False),('s24be',24,24,'A_PCM/INT/BIG',True),('s32le',32,32,'A_PCM/INT/LIT',False),('s32be',32,32,'A_PCM/INT/BIG',True),('f32',32,32,'A_PCM/FLOAT/IEEE',False),('f64',64,64,'A_PCM/FLOAT/IEEE',False),('s16-default',16,0,'A_PCM/INT/LIT',False),('f32-default',32,0,'A_PCM/FLOAT/IEEE',False)]
def generate():
 files={};cases=[]
 for fmt,bits,declared,codec,big in FORMATS:
  for channels in ([1,2]if fmt=='f32'else[1]):
   for label,offset,gap,factor,head_packet in [('offset-head',2400,0,1.,0),('gap-head',0,48,1.,2),('scaled-gap',0,192,32.,2),('delay-gap',0,48,1.,2),('bad-last',0,48,1.,2)]:
    if label=='bad-last'and(fmt!='f32'or channels!=1):continue
    head=0 if factor!=1 else 17;tail=0 if factor!=1 else 13;delay=17 if label=='delay-gap'else 0
    audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',codec.encode())+(ebml('23314f',struct.pack('>d',factor))if factor!=1 else b'')+(u('56aa',(delay*1000000000+24000)//48000)if delay else b'')+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',channels)+(u('6264',declared)if declared else b'')))
    video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
    cluster=40000 if offset else 0;blocks=u('e7',cluster)
    for i in range(6):
     at=i*64+offset+(gap if i>=3 else 0);q=(Fraction(at*1000000000,48000)-cluster*1000)/(1000*Fraction.from_float(factor));relative=q.numerator//q.denominator
     payload=bytearray()
     for n in range(i*64,(i+1)*64):
      for c in range(channels):
       v=value(n,c)
       if codec.endswith('IEEE'):payload+=struct.pack('<f'if bits==32 else '<d',v)
       elif bits==8:payload.append(int(v*128)+128)
       else:payload+=int(v*(1<<(bits-1))).to_bytes(bits//8,'big'if big else'little',signed=True)
     if label=='bad-last'and i==5:payload=payload[:-1]
     padding=-head if i==head_packet else tail if i==5 else 0
     raw=b'\x82'+struct.pack('>hB',relative,0)+payload
     if padding:
      ns=(abs(padding)*1000000000+24000)//48000
      if padding<0:ns=-ns
      audio_block=ebml('a0',ebml('a1',raw)+ebml('75a2',ns.to_bytes(8,'big',signed=True)))
     else:audio_block=ebml('a3',raw)
     blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*2000,0x80)+frame(8,'420',16,16))+audio_block
    data=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3 if factor!=1 else 4)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
    name=f'{label}-{fmt}-{channels}.mkv';files[name]=data;cases.append(dict(file=name,channels=channels,bits=bits,declared_bits=declared,codec=codec,offset=offset,gap=gap,factor=factor,head_packet=head_packet,head=head,tail=tail,delay=delay,samples=384+offset+gap-tail-delay,invalid=label=='bad-last'))
 files['manifest.json']=(json.dumps(dict(cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 ROOT.mkdir(exist_ok=True)
 for name,data in generate().items():(ROOT/name).write_bytes(data)
