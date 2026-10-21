"""Owned WavPack custom-rate vectors, including the four-byte 31-bit form.
No external codec, process, network, private source or generated fixture is read.
"""
import json,struct
from pathlib import Path
from generate_wavpack_false_stereo import block
from generate_wavpack_float import parts,rewrite
from generate_wavpack_multichannel import compact
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-custom-rate'
N=750

def movie(packets,rate,channels):
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_WAVPACK4')+element(0x63a2,b'\x10\x04')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels)+uint(0x6264,16))
 clusters=b'';vp9=frame(8,'420',16,16)
 for i,p in enumerate(packets):
  clusters+=element(0x1f43b675,uint(0xe7,i*N*1_000_000_000//rate)+element(0xa3,b'\x81\0\0\x80'+vp9)+element(0xa3,b'\x82\0\0\x80'+p))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1)+element(0x4489,struct.pack('>d',3*N*1e9/rate)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)

def generate():
 files={};cases=[]
 for channels in [1,2,6]:
  for rate in [37500,0xffffff,0x1000000,0x7fffffff]:
   for width,reserved in [(4,0),(4,0x80000000)]+([(3,0)]if rate<=0xffffff else[]):
    name=f'c{channels}-r{rate}-bytes{width}-reserved{int(bool(reserved))}';frames=[];oracle=b'';packets=[]
    for i in range(3):
     components=[];controls=[]
     for c in range((channels+1)//2):
      raw,pcm=block(16,8,False,37500,i*N,'mono'if channels==1 else'stereo',amplitude=1<<c)
      raw=rewrite(raw,[(t,(rate|reserved).to_bytes(width,'little')if t==0x27 else b)for t,b in parts(raw)])
      flags=int.from_bytes(raw[24:28],'little')&~0x1800
      if c==0:flags|=0x800
      if c==(channels+1)//2-1:flags|=0x1000
      raw=rewrite(raw,parts(raw),flags);components.append(raw);controls.append(pcm)
     expected=b''.join(pcm[row*4*(1 if channels==1 else 2):(row+1)*4*(1 if channels==1 else 2)]for row in range(N)for pcm in controls)
     fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=b''.join(components);files[pn]=compact(components);packets.append(files[pn]);oracle+=expected;frames.append(dict(file=fn,packet=pn,pts=i*N))
    files[name+'.mkv']=movie(packets,rate,channels);files[name+'.f32le']=oracle
    cases.append(dict(name=name,channels=channels,rate=rate,file=name+'.mkv',oracle=name+'.f32le',frames=frames))
 base=cases[0];raw=files[base['frames'][0]['file']];invalid=[]
 for name,payload,message in [('zero-three',bytes(3),'sample rate is zero'),('short',bytes(2),'wrong length'),('long',bytes(5),'wrong length'),('zero',bytes(4),'sample rate is zero'),('reserved-only',(0x80000000).to_bytes(4,'little'),'sample rate is zero')]:
  bad=rewrite(raw,[(t,payload if t==0x27 else b)for t,b in parts(raw)])
  fn=name+'.wv';mv=name+'.mkv';files[fn]=bad;files[mv]=movie([compact([bad])],37500,1);invalid.append(dict(file=fn,movie=mv,message=message))
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,samples=N,cases=cases,invalid=invalid),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
