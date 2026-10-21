"""Owned WavPack mono-coded stereo, integer fixup, and Matroska packet mapping.
All entropy streams are literal zero runs with nonzero predictor seeds.
No external encoder/decoder, subprocess, network, private data or old fixture.
"""
import json,struct,itertools
from pathlib import Path
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-false-stereo'
U32=(1<<32)-1
class Bits:
 def __init__(self):self.bits=[]
 def put(self,n,width):self.bits += [(n>>i)&1 for i in range(width)]
 def run(self,n):
  k=n.bit_length();self.bits += [1]*k+[0]
  if k>1:self.put(n-(1<<(k-1)),k-1)
 def close(self):
  while len(self.bits)%16:self.bits.append(1)
  return bytes(sum(self.bits[i+j]<<j for j in range(8))for i in range(0,len(self.bits),8))
def metadata(tag,body):
 n=(len(body)+1)//2;odd=len(body)%2
 assert n<256
 return bytes([tag|(0x40 if odd else 0),n])+body+(b'\0' if odd else b'')
def log_seed(n):return (1 if n>=0 else -1)*abs(n).bit_length()*256 if n else 0
def i32(n):return (n+(1<<31))%(1<<32)-(1<<31)
def predict(term,negative,seed_scale,n,stereo=False):
 seed=([2,2] if term==17 else [2,1] if term==18 else [1,-2,4,-8,1,2,-4,8][:term])
 seeds=[[v*seed_scale for v in seed]]
 if stereo:seeds.append([-v*2 for v in seeds[0]])
 output=[];history=[v[:]for v in seeds]
 for index in range(n):
  row=[]
  for ch,h in enumerate(history):
   source=2*h[0]-h[1] if term==17 else (3*h[0]-h[1])//2 if term==18 else h[index%term]
   value=-source if negative else source
   if term>8:h[1]=h[0];h[0]=value
   else:h[index%term]=value
   row.append(value)
  output+=row
 return seeds,output
def block(depth,term,negative,rate,index=0,mode='false',tail='none',shift=0,amplitude=1):
 n=rate//50;stereo=mode=='stereo';channels=2 if stereo else 1
 seeds,core=predict(term,negative,amplitude*(1<<(index//n)),n,stereo)
 mag=max(abs(v)for v in core).bit_length()
 flags=(depth//8-1)|(4 if mode=='mono' else 1<<30 if mode=='false' else 0)|(1<<11)|(1<<12)|(shift<<13)|(mag<<18)
 rates=[6000,8000,9600,11025,12000,16000,22050,24000,32000,44100,48000,64000,88200,96000,192000]
 flags|=(rates.index(rate)if rate in rates else 15)<<23
 # The one pass has zero adaptation and weight +/-1024. Power-of-two
 # history values have exactly represented log seeds, independent of tables.
 terms=metadata(2,bytes([term+5]));weights=metadata(3,bytes([128 if negative else 127])*channels)
 taps=([v for seed in seeds for v in seed] if term>8 else [seeds[ch][i]for i in range(term)for ch in range(channels)])
 samples=metadata(4,b''.join(struct.pack('<h',log_seed(v))for v in taps))
 data=terms+weights+samples+metadata(5,bytes(6*channels))
 bits=Bits();bits.run(n*channels);data+=metadata(10,bits.close())
 if rate not in rates:data+=metadata(0x27,rate.to_bytes(3,'little'))
 crc=U32
 for v in core:crc=(crc*3+v)&U32
 expanded=core[:]
 if tail!='none':
  sent=2 if tail.startswith('sent')else 0
  kind=tail.split('-')[-1];run=2 if kind in ['zeros','ones','dups'] else 0
  info=[sent,run if kind=='zeros' else 0,run if kind=='ones' else 0,run if kind=='dups' else 0]
  flags|=1<<8;data+=metadata(9,bytes(info));extra=Bits();extended_crc=U32;expanded=[]
  for i,v in enumerate(core):
   low=(i+index//n)%4
   if sent:extra.put(low,sent);v=(v<<sent)|low
   if kind=='zeros':v<<=run
   elif kind=='ones':v=((v+1)<<run)-1
   elif kind=='dups':v=((v+(v&1))<<run)-(v&1)
   v=i32(v);expanded.append(v)
   extended_crc=(extended_crc*9+((v&0xffff)*3)+((v>>16)&0xffff))&U32
  if sent:data+=metadata(12,struct.pack('<I',extended_crc)+extra.close())
 stored=depth//8;post=(2 if stored<=2 else 4)*8-depth+shift
 full=[i32(v<<post)for v in expanded]
 if mode=='false':full=[v for x in full for v in [x,x]]
 scale=32768 if depth<=16 else 2147483648
 oracle=b''.join(struct.pack('<f',float(v)/scale)for v in full)
 header=b'wvpk'+struct.pack('<IHBBIIIII',24+len(data),0x410,0,0,n*3,index,n,flags,crc)
 return header+data,oracle

def movie(packets,rate,depth):
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+uint(0x23e383,20_000_000)+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_WAVPACK4')+element(0x63a2,b'\x10\x04')+uint(0x23e383,20_000_000)+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,2)+uint(0x6264,depth))
 clusters=b'';vp9=frame(8,'420',16,16)
 for i,p in enumerate(packets):
  compact=p[20:32]+p[32:]
  clusters+=element(0x1f43b675,uint(0xe7,i*20)+element(0xa3,b'\x81\0\0\x80'+vp9)+element(0xa3,b'\x82\0\0\x80'+compact))
 header=element(0x1a45dfa3,element(0x4282,b'matroska'))
 return header+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',60)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)
def generate():
 files={};cases=[]
 for depth,term,negative in itertools.product([8,16,24,32],[1,2,8,17,18],[False,True]):
  if negative and term>8:continue
  for tail in (['none','zeros','ones','dups','sent','sent-zeros','sent-ones','sent-dups']if depth==32 else ['none']):
   rate=[8000,48000,37500][len(cases)%3];shift=(len(cases)%2 if depth==8 else len(cases)%3)
   name=f'd{depth}-t{term}-neg{int(negative)}-{tail}';packets=[];frames=[];oracle=b''
   for i in range(3):
    p,pcm=block(depth,term,negative,rate,i*(rate//50),'false',tail,shift);mono,_=block(depth,term,negative,rate,i*(rate//50),'mono',tail,shift)
    fn=f'{name}-{i}.wv';mn=f'{name}-{i}-mono.wv';files[fn]=p;files[mn]=mono;frames.append(dict(file=fn,mono=mn,pts=i*(rate//50)));packets.append(p);oracle+=pcm
   files[name+'.mkv']=movie(packets,rate,depth);files[name+'.f32le']=oracle
   cases.append(dict(name=name,rate=rate,depth=depth,file=name+'.mkv',oracle=name+'.f32le',frames=frames))
 for depth,term in itertools.product([8,16,24,32],[8,17]):
  for tail in (['none','sent-dups'] if depth==32 else ['none']):
   name=f'large-d{depth}-t{term}-{tail}';rate=8000;packets=[];frames=[];oracle=b''
   amplitude=1<<(depth-7) if depth<32 else 1<<17
   shift=8 if depth==32 and tail=='none' else 0
   for i in range(3):
    p,pcm=block(depth,term,False,rate,i*160,'false',tail,shift,amplitude)
    mono,_=block(depth,term,False,rate,i*160,'mono',tail,shift,amplitude)
    fn=f'{name}-{i}.wv';mn=f'{name}-{i}-mono.wv';files[fn]=p;files[mn]=mono;frames.append(dict(file=fn,mono=mn,pts=i*160));packets.append(p);oracle+=pcm
   files[name+'.mkv']=movie(packets,rate,depth);files[name+'.f32le']=oracle
   cases.append(dict(name=name,rate=rate,depth=depth,file=name+'.mkv',oracle=name+'.f32le',frames=frames))
 # The number of coded channels is per block, not stream state.
 packets=[];frames=[];oracle=b''
 for i,mode in enumerate(['false','stereo','false']):
  p,pcm=block(16,1,False,8000,i*160,mode);fn=f'transition-{i}.wv';files[fn]=p;frames.append(dict(file=fn,pts=i*160));packets.append(p);oracle+=pcm
 files['transition.mkv']=movie(packets,8000,16);files['transition.f32le']=oracle;cases.append(dict(name='transition',rate=8000,depth=16,file='transition.mkv',oracle='transition.f32le',frames=frames))
 bad=[];base=bytearray(files[cases[0]['frames'][0]['file']]);flags=int.from_bytes(base[24:28],'little')
 for name,offset,value,message in [('mono-and-false',24,flags|4,'both mono and false stereo'),('wrong-crc',28,0,'checksum')]:
  p=bytearray(base);p[offset:offset+4]=value.to_bytes(4,'little');fn=name+'.wv';files[fn]=bytes(p);bad.append(dict(file=fn,message=message))
 # Independently corrupt extended-stream CRC, padding and length. Main PCM
 # CRC and all predictor/entropy metadata remain unchanged.
 extended=next(c for c in cases if c['name'].endswith('sent-dups') and c['rate']==37500)
 raw=bytearray(files[extended['frames'][0]['file']]);at=32
 while (raw[at]&0x3f)!=12:at+=2+raw[at+1]*2
 body=at+2
 mutations=[]
 p=bytearray(raw);p[body]^=1;mutations.append(('extra-crc',p,'extra bits do not match their checksum'))
 p=bytearray(raw);p[-1]^=0x80;mutations.append(('extra-padding',p,'padding is not all ones'))
 p=bytearray(raw);p+=b'\xff\xff';p[at+1]+=1;p[4:8]=(len(p)-8).to_bytes(4,'little');mutations.append(('extra-trailing',p,'leaves its extra bits unread'))
 p=bytearray(raw[:-2]);p[at+1]-=1;p[4:8]=(len(p)-8).to_bytes(4,'little');mutations.append(('extra-truncated',p,'ends inside its samples'))
 for name,p,message in mutations:
  fn=name+'.wv';files[fn]=bytes(p);bad.append(dict(file=fn,message=message,rate=37500,depth=32))
 for c in bad:
  rate=c.get('rate',8000);depth=c.get('depth',8)
  tail=[block(depth,1,False,rate,i*(rate//50))[0]for i in [1,2]]
  c['movie']=c['file'][:-3]+'.mkv';files[c['movie']]=movie([files[c['file']]]+tail,rate,depth)
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
