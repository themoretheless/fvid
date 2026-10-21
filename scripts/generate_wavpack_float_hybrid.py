"""Owned hybrid float/WVX controls and unrelated-ID lacing regressions.
No external codec, generated artifact, private input, subprocess or network is read.
"""
import itertools,json,struct
from pathlib import Path
from generate_wavpack_correction import block as corrected_block,movie,compact_correction
from generate_wavpack_false_stereo import block as integer_block,Bits
from generate_wavpack_float import metadata,parts,rewrite,float_crc
from generate_wavpack_multichannel import compact,movie as main_movie
from generate_ffv1_gray_fixtures import element,uint
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-float-hybrid'
MASK=0xffffffff

def floats(values,maximum,shift,scheme,extra,seed):
 bits=Bits();words=[]
 if scheme=='optimized'and extra:bits.put(2,5);bits.put(7,5)
 for i,v in enumerate(values):
  exponent=maximum;sign=int(v<0);magnitude=abs(v)<<shift
  if not extra:
   value=(v<<shift)&MASK;sign=value>>31;magnitude=-(value-(1<<32))if sign else value
   if v==0:exponent=0
   elif magnitude>=0x1000000:
    while magnitude&0xf000000:magnitude>>=1;exponent+=1
   elif exponent:
    moved=0
    while not magnitude&0x800000:
     exponent-=1
     if not exponent:break
     magnitude<<=1;moved+=1
    if scheme=='ones'and moved:magnitude|=(1<<moved)-1
  elif v==0:exponent=0
  else:
   k=magnitude.bit_length()-1;moved=min(23-k,max(0,maximum-1));exponent=max(0,maximum-(23-k));magnitude<<=moved
   if scheme=='ones':magnitude|=(1<<moved)-1
   elif scheme=='same'and moved:
    same=(i+seed)&1;bits.put(same,1)
    if same:magnitude|=(1<<moved)-1
   elif scheme in ['sent','optimized']and moved:
    zeros=max(min(moved,2),max(0,moved-7))if scheme=='optimized'else 0;count=moved-zeros
    low=((i*0x13579+seed*17)&((1<<count)-1))if count else 0;bits.put(low,count);magnitude|=low<<zeros
  words.append((sign<<31)|((exponent&255)<<23)|(magnitude&0x7fffff))
 return words,bits

def generate():
 files={};cases=[]
 for mode,profile,scheme,maximum in itertools.product(['mono','stereo','false'],['snr','bitrate-delta','zero-limit'],['zero','ones','same','sent','optimized'],[25,127,254]):
  name=f'{mode}-{profile}-{scheme}-e{maximum}';channels=1 if mode=='mono'else 2;shift=len(cases)%3;frames=[];full=b'';lossy=b'';mp=[];cp=[]
  for i in range(3):
   main,correction,l,x=corrected_block(32,mode,profile,'none',mode=='stereo',1,i)
   ints=lambda b:[int(v*(1<<31))for(v,)in struct.iter_unpack('<f',b)]
   exact=ints(x);approximate=ints(l)
   if mode=='false':exact=exact[::2];approximate=approximate[::2]
   words,bits=floats(exact,maximum,shift,scheme,True,i);main_words,_=floats(approximate,maximum,shift,scheme,False,i)
   flags=int.from_bytes(main[24:28],'little')|128;tool={'zero':0,'ones':1,'same':2,'sent':4,'optimized':4}[scheme]
   main=rewrite(main,parts(main)+[(8,bytes([tool,shift,maximum,127]))],flags)
   items=parts(correction)
   if bits.bits:items+=[(0x2c if scheme=='optimized'else 12,struct.pack('<I',float_crc(words))+bits.close())]
   correction=rewrite(correction,items,flags)
   fn=f'{name}-{i}.wv';cn=f'{name}-{i}.wvc';pn=f'{name}-{i}.packet';an=f'{name}-{i}.additional';files[fn]=main;files[cn]=correction;files[pn]=compact([main]);files[an]=compact_correction([correction],channels);mp.append(files[pn]);cp.append(files[an]);frames.append(dict(file=fn,correction=cn,packet=pn,additional=an,pts=i*160))
   if mode=='false':words=[w for v in words for w in[v,v]];main_words=[w for v in main_words for w in[v,v]]
   full+=b''.join(struct.pack('<I',w)for w in words);lossy+=b''.join(struct.pack('<I',w)for w in main_words)
  files[name+'.mkv']=movie(mp,cp,channels,32);files[name+'.f32le']=full;files[name+'-lossy.f32le']=lossy
  cases.append(dict(name=name,channels=channels,file=name+'.mkv',oracle=name+'.f32le',lossy=name+'-lossy.f32le',frames=frames))
 # Valid main streams whose lossy core exceeds the lossless mantissa sentinel.
 for mode,amplitude,shift,maximum in itertools.product(['mono','stereo','false'],[1<<23,1<<24,1<<25],[0,1,2],[25,127,254]):
  name=f'overshoot-{mode}-a{amplitude}-s{shift}-e{maximum}';channels=1 if mode=='mono'else 2;frames=[];full=b'';mp=[]
  for i in range(3):
   raw,_=integer_block(32,1,False,8000,i*160,mode,amplitude=amplitude);flags=int.from_bytes(raw[24:28],'little')|8|128
   # Predictor seeds double per frame in the independent integer generator.
   core=[(amplitude<<i)*(1 if c==0 else-2)for _ in range(160)for c in range(2 if mode=='stereo'else 1)]
   words,_=floats(core,maximum,shift,'zero',False,i)
   raw=rewrite(raw,parts(raw)+[(6,bytes([0,4])*(2 if mode=='stereo'else 1)),(8,bytes([0,shift,maximum,127]))],flags)
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=raw;files[pn]=compact([raw]);mp.append(files[pn]);frames.append(dict(file=fn,packet=pn,pts=i*160))
   if mode=='false':words=[w for v in words for w in[v,v]]
   full+=b''.join(struct.pack('<I',w)for w in words)
  files[name+'.mkv']=main_movie(mp,8000,channels,32);files[name+'.f32le']=full
  cases.append(dict(name=name,channels=channels,file=name+'.mkv',oracle=name+'.f32le',lossy=name+'.f32le',frames=frames,main_only=True))
 # Foreign BlockAddID must not disable legal main-stream lacing.
 main=[];extra=[];oracle=b''
 for i in range(3):
  m,c,l,x=corrected_block(16,'mono','snr','none',False,8,i);main.append(compact([m]));extra.append(compact_correction([c],1));oracle+=l+l
 for scheme in ['xiph','fixed','ebml']:
  data=movie(main,extra,1,16,add_id=2,laced=True,lace_kind=scheme,interval_ms=40,duration_ms=120)
  files['foreign-'+scheme+'.mkv']=data
 files['foreign-lace.f32le']=oracle
 base=next(c for c in cases if c.get('main_only'));bad=files[base['frames'][2]['file']];bad=rewrite(bad,parts(bad)+[(12,bytes(4)+b'\xff\xff')]);files['overshoot-with-wvx.wv']=bad;files['overshoot-with-wvx.mkv']=main_movie([compact([bad])],8000,1,32)
 # Choose a sent-mantissa stream so side CRC corruption reaches float reconstruction.
 crc_case=next(c for c in cases if c['name']=='mono-snr-sent-e127');f=crc_case['frames'][0];raw=files[f['correction']];corrupted=rewrite(raw,[(t,bytes([b[0]^1])+b[1:]if t==12 else b)for t,b in parts(raw)])
 files['float-crc.wvc']=corrupted;files['float-crc.mkv']=movie([files[f['packet']]],[compact_correction([corrupted],1)],1,32)
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,crc_case=crc_case['name']),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
