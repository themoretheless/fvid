"""Owned DSD raw/static/adaptive range streams and literal interleaved byte oracles.
Scalar 56-tap PCM controls retain filter history across the three short frames.
No external codec, process, network, private input or generated fixture is read.
"""
import json,struct,itertools,math
from pathlib import Path
from generate_wavpack_float import metadata,rewrite,parts
from generate_wavpack_multichannel import compact
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-dsd'
MASK=0xffffffff;N=128
TAPS=[4,17,56,147,336,692,1315,2337,3926,6281,9631,14216,20275,28021,37619,49155,62616,77870,94649,112551,131049,149507,167220,183448,197472,208636,216402,220385,220385,216402,208636,197472,183448,167220,149507,131049,112551,94649,77870,62616,49155,37619,28021,20275,14216,9631,6281,3926,2337,1315,692,336,147,56,17,4]
def i32(v):return (v+(1<<31))%(1<<32)-(1<<31)
def f(v):return struct.unpack('<f',struct.pack('<f',v))[0]
class Range:
 def __init__(self):self.low=0;self.high=MASK;self.data=bytearray()
 def normalize(self):
  while ((self.low^self.high)&0xff000000)==0:self.data.append(self.high>>24);self.low=(self.low<<8)&MASK;self.high=((self.high<<8)|255)&MASK
 def finish(self):return bytes(self.data)+self.low.to_bytes(4,'big')
def fast(values,stereo,history,rle):
 bins=1<<history
 probabilities=[[0]*256 for _ in range(bins)]if rle else[[1]*256 for _ in range(bins)]
 if rle:
  for p in probabilities:
   for code,v in zip([0,85,170,255],[1,2,3,4]):p[code]=v
 cumulative=[[0]+list(itertools.accumulate(p))for p in probabilities]
 raw=bytes(v for p in probabilities for v in p)
 if not rle:table=bytes([history,255])+raw
 else:
  out=bytearray();at=0
  while at<len(raw):
   if raw[at]:out.append(raw[at]);at+=1
   else:
    end=at
    while end<len(raw)and raw[end]==0 and end-at<251:end+=1
    out.append(4+end-at);at=end
  table=bytes([history,4])+out+b'\0'
 coder=Range();p0=p1=0
 for code in values:
  sums=cumulative[p0];step=(coder.high-coder.low)//sums[-1]
  if not step:coder.data+=coder.low.to_bytes(4,'big');coder.low=0;coder.high=MASK;step=MASK//sums[-1]
  coder.low+=sums[code]*step;coder.high=coder.low+probabilities[p0][code]*step-1;coder.normalize()
  if stereo:p0=p1;p1=code&(bins-1)
  else:p0=code&(bins-1)
 return table+coder.finish()
def ptable(rate):
 value=0x808000;rate<<=8;table=[0]*256
 for _ in range((rate+128)>>8):value+=(0x10000-value)>>8
 for j in range(128):
  table[j]=value;table[255-j]=0x100ffff-value
  if value>0x10000:
   rate+=(rate*20+128)>>8
   for _ in range((rate+64)>>7):value+=(0x10000-value)>>8
 return table
def high(values,channels,seed):
 rate=[0,7,255][seed%3];table=ptable(rate);header=bytearray([rate,20]);states=[]
 for c in range(channels):
  init=[128+(seed+c+i)%7-3 for i in range(5)];factor=-64 if c==0 else 64
  header+=bytes(init)+struct.pack('<h',factor);states.append(dict(filters=[v<<12 for v in init]+[0],factor=factor))
 def value(s):
  a=s['filters'];return i32(i32(a[0]-a[4])+(i32(a[5]*s['factor'])>>2))
 coder=Range()
 for at in range(0,len(values),channels):
  for c,s in enumerate(states):s['value']=value(s)
  for bit in range(7,-1,-1):
   for c,s in enumerate(states):
    one=(values[at+c]>>bit)&1;slot=(s['value']>>8)&255;split=coder.low+((coder.high-coder.low)>>8)*(table[slot]>>16)
    if one:coder.high=split;table[slot]+=(0x10000fe-table[slot])>>8
    else:coder.low=split+1;table[slot]+=(0x10000-table[slot])>>8
    coder.normalize();a=s['filters'];b=-one;v=i32(s['value']+i32(a[5]*8))
    s['factor']=i32(s['factor']+((((v^b)>>31)|1)&((v^i32(v-i32(a[5]*16)))>>31)))
    a[0]=i32(a[0]+(i32((b&0x100000)-a[0])>>6));a[1]=i32(a[1]+(i32((b&0x100000)-a[1])>>4))
    a[2]=i32(a[2]+(i32(a[1]-a[2])>>4));a[3]=i32(a[3]+(i32(a[2]-a[3])>>4));d=i32(a[3]-a[4])>>4
    a[4]=i32(a[4]+d);a[5]=i32(a[5]+(i32(d-a[5])>>3));s['value']=value(s)
  for s in states:s['factor']=i32(s['factor']-((s['factor']+512)>>10))
 return bytes(header)+coder.finish()
def signal(pattern,at,c):
 if pattern=='silence':return 85
 if pattern=='square':return 255 if ((at+c*7)//8)%2 else 0
 if pattern=='ramp':return (at*37+c*59)&255
 return (((at+c*101+7)*1103515245+12345)&MASK)>>16&255

def block(mode,kind,power,pattern,index,channel,history,n=N):
 nc=2 if kind=='stereo'else 1
 values=[signal(pattern,index*n+i,channel+c)for i in range(n)for c in range(nc)]
 if mode=='rle':values=[[0,85,170,255][v%4]for v in values]
 body=bytes([power,0])+bytes(values)if mode=='raw'else bytes([power,1])+fast(values,nc==2,history,mode=='rle')if mode in ['fast','rle']else bytes([power,3])+high(values,nc,index+channel)
 flags=0x80000000|0x1800|(9<<23)|(4 if kind=='mono'else(1<<30)if kind=='false'else 0)
 crc=MASK
 for v in values:crc=(crc*3+v)&MASK
 data=metadata(14,body)
 raw=b'wvpk'+struct.pack('<IHBBIIIII',24+len(data),0x410,0,0,n*3,index*n,n,flags,crc)+data
 if kind=='false':values=[v for value in values for v in[value,value]]
 return raw,bytes(values)
def pcm(data,channels,n=N):
 scale=((1<<23)-1)/sum(TAPS)*16;weights=[math.floor(v*scale+0.5)for v in TAPS];history=[[85]*7 for _ in range(channels)];all=[]
 for frame in range(3):
  out=[]
  for i in range(n):
   for c in range(channels):
    h=history[c];h.pop(0);h.append(data[(frame*n+i)*channels+c]);total=sum(w if h[j//8]&(0x80>>(j%8))else-w for j,w in enumerate(weights));out.append(f((total+8)>>4))
  if frame==0 and n>=16:
   maximum=min(10,(n-6)//2)
   for c in range(channels):
    left=right=f(0)
    for period in range(5,maximum+1):
     first=second=f(0)
     for i in range(period):first=f(first+f(out[(6+i)*channels+c]/period));second=f(second+f(out[(6+period+i)*channels+c]/period))
     lratio=f(f(6+f(period/2))/period);rratio=f(f(period/2)/period);diff=f(first-second)
     left=f(left+f(first+diff*lratio));right=f(right+f(first+diff*rratio))
    left=f(left/(maximum-4));right=f(right/(maximum-4));slope=f(f(right-left)/5)
    for i in range(6):out[i*channels+c]=f(int(f(left+i*slope)+0.5))
  all+=out
 return b''.join(struct.pack('<f',f(v/(1<<23)))for v in all)
def movie(packets,rate,channels,n=N):
 step=n*1_000_000_000/rate
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_WAVPACK4')+element(0x63a2,b'\x10\x04')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels)+uint(0x6264,1))
 clusters=b'';vp9=frame(8,'420',16,16)
 for i,p in enumerate(packets):clusters+=element(0x1f43b675,uint(0xe7,i*n*1_000_000_000//rate)+element(0xa3,b'\x81\0\0\x80'+vp9)+element(0xa3,b'\x82\0\0\x80'+p))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1)+element(0x4489,struct.pack('>d',step*len(packets))))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)
def generate():
 files={};cases=[]
 specs=[(128,)+x for x in itertools.product(['raw','fast','rle','high'],['mono','stereo','false','six'],[0,3,4],['silence','square','ramp','noise'])]
 specs += [(n,mode,layout,3,'noise')for n,mode,layout in itertools.product([1,7,15,16],['raw','fast','rle','high'],['mono','stereo','false','six'])]
 for n,mode,layout,power,pattern in specs:
  name=(f'short{n}-'if n!=128 else'')+f'{mode}-{layout}-p{power}-{pattern}';kinds=['stereo','mono','mono','false']if layout=='six'else[layout];channels=6 if layout=='six'else 1 if layout=='mono'else 2;history=[0,1,5][len(cases)%3];rate=44100<<power;frames=[];oracle=b'';packets=[]
  for i in range(3):
   components=[];controls=[];channel=0
   for group,kind in enumerate(kinds):
    raw,values=block(mode,kind,power,pattern,i,channel,history,n);width=1 if kind=='mono'else 2
    flags=int.from_bytes(raw[24:28],'little')&~0x1800
    if group==0:flags|=0x800
    if group==len(kinds)-1:flags|=0x1000
    mask=63 if channels==6 else 4 if channels==1 else 3
    raw=rewrite(raw,parts(raw)+([(13,bytes([channels])+mask.to_bytes(4,'little'))]if group==0 else[]),flags)
    components.append(raw);controls.append((values,width));channel+=width
   decoded=b''.join(v[row*w:(row+1)*w]for row in range(n)for v,w in controls)
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=b''.join(components);files[pn]=compact(components);packets.append(files[pn]);oracle+=decoded;frames.append(dict(file=fn,packet=pn,pts=i*n))
  files[name+'.dsd']=oracle;files[name+'.f32le']=pcm(oracle,channels,n);files[name+'.mkv']=movie(packets,rate,channels,n)
  cases.append(dict(name=name,rate=rate,channels=channels,samples=n,file=name+'.mkv',oracle=name+'.dsd',pcm=name+'.f32le',frames=frames))
 invalid=[];base=cases[0];raw=files[base['frames'][0]['file']];items=parts(raw)
 def bad(name,body,message,flags=None):
  payload=rewrite(raw,[(t,body if t==14 else b)for t,b in items],flags);fn=name+'.wv';mv=name+'.mkv';files[fn]=payload;files[mv]=movie([compact([payload])],44100,1);invalid.append(dict(file=fn,movie=mv,message=message))
 original=next(b for t,b in items if t==14)
 bad('metadata-short',bytes(1),'metadata is too short')
 bad('pcm-flags',original,'incompatible PCM coding flags',int.from_bytes(raw[24:28],'little')|256)
 bad('missing-flag',original,'DSD metadata without DSD coding',int.from_bytes(raw[24:28],'little')&~0x80000000)
 bad('probability-bound',bytes([0,1,0,255])+bytes([255])*256+bytes(4),'probability storage exceeds')
 bad('mode',bytes([0,2])+original[2:],'coding mode is invalid')
 bad('multiplier',bytes([32])+original[1:],'multiplier exceeds')
 bad('rate-overflow',bytes([31])+original[1:],'sample rate overflows')
 bad('raw-short',original[:-1],'raw DSD sample count mismatch')
 bad('high-rate',bytes([0,3,0,19])+bytes(20),'adaptation rate is invalid')
 bad('high-short',bytes([0,3,0,20])+bytes(3),'stream is truncated')
 bad('fast-history',bytes([0,1,6,255])+bytes(20),'history exceeds')
 bad('fast-empty',bytes([0,1,0,255])+bytes(260),'empty probability context')
 bad('rle-short',bytes([0,1,0,4,0]),'probability table is incomplete')
 payload=bytearray(raw);payload[28]^=1;files['crc.wv']=bytes(payload);files['crc.mkv']=movie([compact([bytes(payload)])],44100,1);invalid.append(dict(file='crc.wv',movie='crc.mkv',message='DSD checksum'))
 atomic=next(c for c in cases if c['name']=='high-six-p3-noise');raw=files[atomic['frames'][1]['file']];components=[];at=0
 while at<len(raw):length=int.from_bytes(raw[at+4:at+8],'little')+8;components.append(raw[at:at+length]);at+=length
 last=bytearray(components[-1]);last[28]^=1;components[-1]=bytes(last);files['last-crc.wv']=b''.join(components)
 components[0]=rewrite(components[0],[(t,bytes([6])+(0x60f).to_bytes(4,'little')if t==13 else b)for t,b in parts(components[0])]);files['uncommitted-mask.wv']=b''.join(components)
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,samples=N,cases=cases,invalid=invalid,atomic_case=atomic['name']),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
