"""Owned WavPack hybrid main streams: explicit quantizer intervals and scalar PCM.
No external codec, private input, generated fixture, process or network is used.
"""
import math,json,struct,itertools
from pathlib import Path
from generate_wavpack_false_stereo import Bits,block as zero_block
from generate_wavpack_float import parts,rewrite,metadata
from generate_wavpack_multichannel import compact,movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-hybrid'
MASK=0xffffffff

def exp(log):
 if log<0:return -exp(-log)
 m=256+round(256*(2**((log%256)/256)-1));k=log//256-9
 return (m<<k if k>=0 else m>>-k)&MASK

def logarithm(n):
 n+=n>>9;b=n.bit_length();index=(n<<(9-b)if b<9 else n>>(b-9))&255
 return b*256+round(256*math.log2(1+index/256))

def truncated(bits,value,maximum):
 if maximum==0:return
 width=maximum.bit_length();extras=(1<<width)-maximum-1
 if value<extras:bits.put(value,width-1)
 else:
  value+=extras;bits.put(value>>1,width-1);bits.put(value&1,1)

def block(depth,mode,profile,frame,shift=0,tail='none',joint=False,entropy_log=15*256):
 rate=8000;n=160;nc=2 if mode=='stereo'else 1
 bitrate=profile!='snr';balance=profile.startswith('balance');delta=profile.endswith('delta')
 if profile=='zero-limit':acc_override=[8192,8192]
 elif profile in ['balance-high','balance-low']:acc_override=[128,0]
 else:acc_override=None
 slow=[exp(20*256),exp(21*256)];acc=[(1792 if bitrate else 1024)<<16,(1280 if bitrate else 768)<<16]
 if acc_override:acc=[v<<16 for v in acc_override]
 if profile=='balance-low':slow.reverse()
 deltas=[exp(13*256),-exp(12*256)]if delta else[0,0]
 prof=(b''.join(struct.pack('<H',(21-c if profile=='balance-low'else 20+c)*256)for c in range(nc))if bitrate else b'')+b''.join(struct.pack('<H',acc[c]>>16)for c in range(nc))
 if delta:prof+=b''.join(struct.pack('<h',13*256 if c==0 else -12*256)for c in range(nc))
 med=[[exp(entropy_log)]*3 for c in range(nc)];bits=Bits();core=[];trace=[]
 def band(c,k):return (med[c][k]>>4)+1
 def adjust(c,k,up):
  divisor=128>>k;v=med[c][k]
  med[c][k]+=((v+divisor)//divisor*5 if up else -((v+divisor-2)//divisor*2))
 limits=[0,0]
 for slot in range(n*nc):
  c=slot%nc
  if c==0:
   rates=[]
   for ch in range(nc):acc[ch]=(acc[ch]+deltas[ch])&MASK;rates.append(acc[ch]>>16)
   levels=[(s+128)>>8 for s in slow]
   if balance and nc==2:
    b=(levels[1]-levels[0]+rates[1]+1)//2
    rates=([0,rates[0]*2]if b>rates[0]else[rates[0]*2,0]if -b>rates[0]else[rates[0]-b,rates[0]+b])
   for ch in range(nc):
    log=levels[ch]-rates[ch]+256 if bitrate else rates[ch];limits[ch]=exp(log)if log>0 else 0
  # Every explicit even unary token forces the next slot to zone zero.
  zone=0 if slot%2 else (slot//2+frame)%5
  if slot%2==0:bits.put((1<<(2*zone))-1,2*zone);bits.put(0,1)
  low=0
  for k in range(min(zone,2)):low+=band(c,k);adjust(c,k,True)
  k=min(zone,2)
  if zone>2:low+=(zone-2)*band(c,2)
  high=low+band(c,k)-1;adjust(c,k,zone>2)
  branch=[]
  if limits[c]==0:
   value=low+(slot*7+frame)%(high-low+1);truncated(bits,value-low,high-low);mid=value
  else:
   while high-low>limits[c]:
    mid=(low+high+1)//2;choice=(slot+len(branch)+frame)%2;bits.put(choice,1);branch.append(choice)
    if choice:low=mid
    else:high=mid-1
   mid=(low+high+1)//2
  sign=(slot//3+frame)%2;bits.put(sign,1);core.append(~mid if sign else mid)
  if bitrate:slow[c]-=(slow[c]+128)>>8;slow[c]+=logarithm(mid)
  trace.append([zone,limits[c],mid,len(branch)])
 flags=(depth//8-1)|8|0x1800|(1<<23)|(shift<<13)|(31<<18)
 if mode=='mono':flags|=4
 if mode=='false':flags|=1<<30
 if bitrate:flags|=512
 if balance:flags|=1024
 if joint:flags|=16
 data=metadata(2,b'\x06')+metadata(3,bytes(nc))+metadata(4,bytes(2*nc))+metadata(5,b''.join(struct.pack('<H',entropy_log)for _ in range(3*nc)))+metadata(6,prof)+metadata(10,bits.close())
 if tail!='none':flags|=256;data+=metadata(9,bytes([3 if tail=='missing'else 0,2 if tail in ['missing','zeros']else 0,2 if tail=='ones'else 0,2 if tail=='dups'else 0]))
 signal=core[:]
 if joint:
  for i in range(0,len(signal),2):signal[i+1]-=signal[i]>>1;signal[i]+=signal[i+1]
 flags=(flags&~(31<<18))|(max(abs(v)for v in signal).bit_length()<<18)
 crc=MASK
 for v in signal:crc=(crc*3+v)&MASK
 samples=[]
 run=2 if tail!='none'else 0;extra=3 if tail=='missing'else 0
 effective_shift=shift
 if extra:effective_shift+=extra+run;run=0
 elif depth==32:
  moved=min(run,8-shift);run-=moved;effective_shift+=moved
 for v in signal:
  bit=1 if tail=='ones'else v&1 if tail=='dups'else 0
  if run:v=((v+bit)<<run)-bit
  v=max(-(1<<(depth-1))>>effective_shift,min(((1<<(depth-1))-1)>>effective_shift,v))<<effective_shift
  if mode=='false':samples.extend([v,v])
  else:samples.append(v)
 pcm=b''.join(struct.pack('<f',v/(1<<(depth-1)))for v in samples)
 raw=b'wvpk'+struct.pack('<IHBBIIIII',24+len(data),0x410,0,0,n*3,frame*n,n,flags,crc)+data
 return raw,pcm,trace

def generate():
 files={};cases=[]
 for depth,mode,profile in itertools.product([8,16,24,32],['mono','stereo','false'],['snr','bitrate','bitrate-delta','balance','balance-delta','zero-limit','balance-high','balance-low']):
  name=f'{depth}-{mode}-{profile}';frames=[];oracle=b'';packets=[]
  shift=1 if depth in [16,24]else 0;tail=['none','missing','zeros','ones','dups'][len(cases)%5];joint=mode=='stereo'and len(cases)%2==0
  for i in range(3):
   raw,pcm,trace=block(depth,mode,profile,i,shift,tail,joint)
   if len(cases)%3==0:raw=rewrite(raw,parts(raw)+[(7,bytes(4*(2 if mode=='stereo'else 1)))],int.from_bytes(raw[24:28],'little')|64)
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=raw;files[pn]=compact([raw]);files[f'{name}-{i}.trace.json']=(json.dumps(trace)+'\n').encode();oracle+=pcm;packets.append(files[pn]);frames.append(dict(file=fn,packet=pn,pts=i*160))
  files[name+'.mkv']=movie(packets,8000,1 if mode=='mono'else 2,depth);files[name+'.f32le']=oracle
  cases.append(dict(name=name,file=name+'.mkv',oracle=name+'.f32le',channels=1 if mode=='mono'else 2,frames=frames))
 # Run-coded zeros retain nonzero predictor histories; slow levels decay
 # without advancing bitrate accumulators during the zero run.
 for mode,profile in itertools.product(['mono','stereo','false'],['bitrate','bitrate-delta']):
  name=f'zero-run-{mode}-{profile}';frames=[];oracle=b'';packets=[];nc=2 if mode=='stereo'else 1
  for i in range(3):
   raw,pcm=zero_block(16,8,False,8000,i*160,mode)
   prof=b''.join(struct.pack('<H',20*256)for _ in range(nc))+b''.join(struct.pack('<H',1792)for _ in range(nc))
   if profile.endswith('delta'):prof+=b''.join(struct.pack('<h',13*256)for _ in range(nc))
   raw=rewrite(raw,parts(raw)+[(6,prof)],int.from_bytes(raw[24:28],'little')|8|512)
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=raw;files[pn]=compact([raw]);frames.append(dict(file=fn,packet=pn,pts=i*160));oracle+=pcm;packets.append(files[pn])
  files[name+'.mkv']=movie(packets,8000,1 if mode=='mono'else 2,16);files[name+'.f32le']=oracle
  cases.append(dict(name=name,file=name+'.mkv',oracle=name+'.f32le',channels=1 if mode=='mono'else 2,frames=frames))
 for profile in ['snr','bitrate-delta','balance']:
  name='multichannel-'+profile;frames=[];oracle=b'';packets=[]
  for i in range(3):
   components=[];controls=[]
   for c,mode in enumerate(['stereo','mono','mono','false']):
    raw,pcm,_=block(16,mode,profile,i,tail='none')
    flags=int.from_bytes(raw[24:28],'little')&~0x1800
    if c==0:flags|=0x800
    if c==3:flags|=0x1000
    items=parts(raw)+([(13,bytes([6])+bytes([63,0,0,0]))]if c==0 else[])
    components.append(rewrite(raw,items,flags));controls.append((pcm,1 if mode=='mono'else 2))
   pcm=b''.join(p[row*width*4:(row+1)*width*4]for row in range(160)for p,width in controls)
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=b''.join(components);files[pn]=compact(components);frames.append(dict(file=fn,packet=pn,pts=i*160));oracle+=pcm;packets.append(files[pn])
  files[name+'.mkv']=movie(packets,8000,6,16);files[name+'.f32le']=oracle;cases.append(dict(name=name,file=name+'.mkv',oracle=name+'.f32le',channels=6,frames=frames))
 invalid=[];base=cases[0];raw=files[base['frames'][0]['file']]

 for name,items,message in [('profile-short',[(t,b[:-1]if t==6 else b)for t,b in parts(raw)],'profile is the wrong length'),('profile-missing',[(t,b)for t,b in parts(raw)if t!=6],'profile is the wrong length'),('entropy-short',[(t,b[:1]if t==10 else b)for t,b in parts(raw)],'ends inside'),('profile-exponent',[(t,bytes([255,127])if t==6 else b)for t,b in parts(raw)],'exponent exceeds'),('correction',parts(raw)+[(11,bytes(1))],'ends inside')]:
  bad=rewrite(raw,items);fn=name+'.wv';mv=name+'.mkv';files[fn]=bad;files[mv]=movie([compact([bad])],8000,1,8);invalid.append(dict(file=fn,movie=mv,message=message))
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=invalid),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
