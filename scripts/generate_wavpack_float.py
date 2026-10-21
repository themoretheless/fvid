"""Owned IEEE-754 WavPack vectors: literal float words and zero-residual predictors.
No external codec, process, network, private sample or generated fixture is read.
"""
import struct,json,itertools
from pathlib import Path
from generate_wavpack_false_stereo import Bits,log_seed,predict,movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-float'
MASK=(1<<32)-1
def metadata(tag,body):
 words=(len(body)+1)//2;tag|=0x40 if len(body)%2 else 0
 header=bytes([tag,words]) if words<256 else bytes([tag|0x80])+words.to_bytes(3,'little')
 return header+body+(b'\0' if len(body)%2 else b'')
def float_crc(words):
 crc=MASK
 for w in words:crc=(crc*27+(w&0x7fffff)*9+((w>>23)&255)*3+(w>>31))&MASK
 return crc

def block(mode,rate,frame,maximum,shift,scheme,family='normal',optimized=(3,5),norm=127):
 n=rate//50;channels=2 if mode=='stereo' else 1
 if family=='normal':
  term=8;seeds,core=predict(term,False,1<<frame,n,channels==2)
 elif family=='zero':term=1;seeds=[[0]for _ in range(channels)];core=[0]*(n*channels)
 else:
  term=1;seeds=[[1<<24]]+([[-(1<<24)]] if channels==2 else []);core=[s[0]for _ in range(n)for s in seeds]
 flags=3|0x80|(1<<11)|(1<<12)|((1<<30)if mode=='false'else 0)|((1 if rate==8000 else 10 if rate==48000 else 15)<<23)
 flags|=max(abs(v)for v in core).bit_length()<<18
 taps=[seeds[c][i]for i in range(term)for c in range(channels)]
 data=metadata(2,bytes([term+5]))+metadata(3,bytes([127])*channels)+metadata(4,b''.join(struct.pack('<h',log_seed(v))for v in taps))+metadata(5,bytes(6*channels))
 residual=Bits();residual.run(len(core));data+=metadata(10,residual.close())
 if rate==37500:data+=metadata(0x27,rate.to_bytes(3,'little'))
 bits=Bits();words=[]
 tool={'zero':0,'ones':1,'same':2,'sent':4,'optimized':4}[scheme]
 if family=='zero':tool=8|16
 if family=='exception':tool=32
 new=scheme=='optimized'
 if new:bits.put(optimized[0],5);bits.put(optimized[1],5)
 for i,v in enumerate(core):
  if family=='zero':
   # Only values which round down to zero at this maximum exponent are
   # carried as false zeros. Exponent <25 has no explicit exponent field.
   choices=[0,0x80000000]
   if maximum>=24:choices += [1,0x80000001,0x7fffff,0x807fffff]
   if maximum>=25:choices += [0x00800000,0x80800001]
   if maximum>=127:choices += [0x33000001,0xb3000001]
   if maximum>=254:choices += [0x3f000001,0xbf000001]
   word=choices[(i+frame)%len(choices)]
   if word&0x7fffffff:
    bits.put(1,1);bits.put(word&0x7fffff,23)
    if maximum>=25:bits.put((word>>23)&255,8)
    bits.put(word>>31,1)
   else:bits.put(0,1);bits.put(word>>31,1)
  elif family=='exception':
   mantissa=[0,1,0x400001,0x7fffff][(i+frame)%4]
   bits.put(int(mantissa!=0),1)
   if mantissa:bits.put(mantissa,23)
   word=(int(v<0)<<31)|0x7f800000|mantissa
  else:
   magnitude=abs(v)<<shift;k=magnitude.bit_length()-1
   move=min(23-k,max(0,maximum-1))
   exponent=maximum-(23-k) if maximum>23-k else 0
   width=move;mask=(1<<width)-1;low=0
   if scheme=='ones':low=mask
   elif scheme=='same' and width:
    same=(i+frame)&1;bits.put(same,1);low=mask if same else 0
   elif scheme in ['sent','optimized'] and width:
    zeros=max(max(0,width-optimized[1]) if new and optimized[1] else 0,min(width,optimized[0]) if new else 0)
    count=width-zeros;sent=((i*0x13579+frame*17)&((1<<count)-1)) if count else 0
    bits.put(sent,count);low=sent<<zeros
   word=(int(v<0)<<31)|(exponent<<23)|(((magnitude<<move)|low)&0x7fffff)
  words.append(word)
 data+=metadata(8,bytes([tool,shift,maximum,norm]))
 if bits.bits:data+=metadata(0x2c if new else 12,struct.pack('<I',float_crc(words))+bits.close())
 crc=MASK
 for v in core:crc=(crc*3+v)&MASK
 raw=b'wvpk'+struct.pack('<IHBBIIIII',24+len(data),0x410,0,0,n*3,frame*n,n,flags,crc)+data
 output=words if mode=='stereo' else [w for x in words for w in [x,x]]
 return raw,b''.join(struct.pack('<I',w)for w in output),words

def parts(raw):
 result=[];at=32
 while at<len(raw):
  tag=raw[at];h=4 if tag&128 else 2
  words=int.from_bytes(raw[at+1:at+h],'little');length=words*2-(1 if tag&64 else 0)
  result.append((tag&63,raw[at+h:at+h+length]));at+=h+words*2
 assert at==len(raw)
 return result
def rewrite(raw,items,flags=None):
 data=b''.join(metadata(tag,body)for tag,body in items);header=bytearray(raw[:32]);header[4:8]=(24+len(data)).to_bytes(4,'little')
 if flags is not None:header[24:28]=flags.to_bytes(4,'little')
 return bytes(header)+data

def integer_block(mode,rate,frame,duplicate):
 raw,_,_=block(mode,rate,frame,127,0,'zero')
 n=rate//50;_,core=predict(8,False,1<<frame,n,mode=='stereo')
 extra=Bits();extra.put(3,5);words=[]
 for i,v in enumerate(core):
  # Literal maximum width 3 retains two, one or no sent bits depending on
  # coded magnitude. All omitted low bits are authored zeros.
  width=(v if v>=0 else ~v).bit_length()+2
  count=max(0,2-max(0,width-3));low=(i+frame)&((1<<count)-1)
  extra.put(low,count);value=(v<<2)|(low<<(2-count))
  if duplicate:value=((value+(value&1))<<2)-(value&1)
  words.append(value)
 crc=MASK
 for v in words:crc=(crc*9+(v&65535)*3+((v>>16)&65535))&MASK
 items=[p for p in parts(raw)if p[0]!=8]
 items += [(9,bytes([2,0,0,2 if duplicate else 0])),(0x2c,struct.pack('<I',crc)+extra.close())]
 flags=(int.from_bytes(raw[24:28],'little')&~0x80)|0x100
 raw=rewrite(raw,items,flags)
 output=words if mode=='stereo'else[v for w in words for v in [w,w]]
 return raw,b''.join(struct.pack('<f',v/2147483648)for v in output)

def generate():
 files={};cases=[]
 def emit(name,mode,maximum,shift,scheme,family='normal',optimized=(3,5)):
  rate=[8000,48000,37500][len(cases)%3];frames=[];packets=[];oracle=b''
  for i in range(3):
   p,pcm,words=block(mode,rate,i,maximum,shift,scheme,family,optimized,[127,142,0][len(cases)%3]);fn=f'{name}-{i}.wv';files[fn]=p
   entry=dict(file=fn,pts=i*(rate//50))
   if mode=='false':
    mono=bytearray(p);flags=int.from_bytes(mono[24:28],'little');mono[24:28]=((flags&~(1<<30))|4).to_bytes(4,'little');mn=f'{name}-{i}-mono.wv';files[mn]=bytes(mono);entry['mono']=mn
   frames.append(entry);packets.append(p);oracle+=pcm
  files[name+'.mkv']=movie(packets,rate,32);files[name+'.f32le']=oracle
  cases.append(dict(name=name,file=name+'.mkv',oracle=name+'.f32le',rate=rate,frames=frames,family=family))
 for mode,maximum,shift,scheme in itertools.product(['false','stereo'],[0,1,8,24,25,127,150,254],[0,8],['zero','ones','same','sent','optimized']):
  emit(f'{mode}-e{maximum}-s{shift}-{scheme}',mode,maximum,shift,scheme)
 for mode,maximum in itertools.product(['false','stereo'],[0,24,25,127,254]):emit(f'{mode}-zeros-e{maximum}',mode,maximum,0,'sent','zero')
 for mode in ['false','stereo']:emit(mode+'-exceptions',mode,127,0,'sent','exception')
 for mode,limits in itertools.product(['false','stereo'],[(0,0),(3,0),(0,5),(23,1)]):emit(f'{mode}-optimized-{limits[0]}-{limits[1]}',mode,127,0,'optimized',optimized=limits)
 for mode,duplicate in itertools.product(['false','stereo'],[False,True]):
  name=f'optimized-int-{mode}-dup{int(duplicate)}';rate=8000;packets=[];frames=[];oracle=b''
  for i in range(3):
   raw,pcm=integer_block(mode,rate,i,duplicate);fn=f'{name}-{i}.wv';files[fn]=raw;entry=dict(file=fn,pts=i*160)
   if mode=='false':
    flags=(int.from_bytes(raw[24:28],'little')&~(1<<30))|4;mono=rewrite(raw,parts(raw),flags);mn=f'{name}-{i}-mono.wv';files[mn]=mono;entry['mono']=mn
   frames.append(entry);packets.append(raw);oracle+=pcm
  files[name+'.mkv']=movie(packets,rate,32);files[name+'.f32le']=oracle
  cases.append(dict(name=name,file=name+'.mkv',oracle=name+'.f32le',rate=rate,frames=frames,family='integer'))
 bad=[]
 source=next(c for c in cases if c['name']=='false-e127-s0-sent');raw=files[source['frames'][0]['file']];items=parts(raw)
 padding=next(c for c in cases if c['name']=='false-optimized-23-1');padraw=files[padding['frames'][0]['file']]
 def invalid(name,reference,blob,message):
  file=name+'.wv';video=name+'.mkv';files[file]=blob
  files[video]=movie([blob]+[files[f['file']]for f in reference['frames'][1:]],reference['rate'],32)
  bad.append(dict(file=file,movie=video,message=message))
 for name,change,message in [
  ('float-crc',lambda b:bytes([b[0]^1])+b[1:],'float CRC'),
  ('float-truncated',lambda b:b[:-2],'ends inside its samples'),
  ('float-extra-word',lambda b:b+b'\xff\xff','leaves its extra bits unread')]:
  invalid(name,source,rewrite(raw,[(t,change(b)if t==12 else b)for t,b in items]),message)
 invalid('float-padding',padding,rewrite(padraw,[(t,b[:-1]+bytes([b[-1]^128])if t==0x2c else b)for t,b in parts(padraw)]),'padding is not all ones')
 invalid('float-info-short',source,rewrite(raw,[(t,b[:3]if t==8 else b)for t,b in items]),'float information must contain four bytes')
 invalid('float-info-missing',source,rewrite(raw,[(t,b)for t,b in items if t!=8]),'float information must contain four bytes')
 invalid('float-width',source,rewrite(raw,items,int.from_bytes(raw[24:28],'little')&~3),'requires 32-bit storage')
 invalid('float-and-int',source,rewrite(raw,items,int.from_bytes(raw[24:28],'little')|256),'requires 32-bit storage')
 invalid('float-shift-overflow',source,rewrite(raw,[(t,bytes([b[0],31,b[2],b[3]])if t==8 else b)for t,b in items]),'float core exceeds mantissa range')
 invalid('mixed-extended-streams',source,rewrite(raw,items+[(0x2c,bytes(6))]),'mixed extended bitstream formats')
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
