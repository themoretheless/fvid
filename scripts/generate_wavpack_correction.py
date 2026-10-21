"""Owned hybrid/correction pairs with scalar predictor and shaping histories.
No external codec, process, network, private source or generated artifact is read.
"""
import itertools,json,struct,copy
from pathlib import Path
from generate_wavpack_hybrid import block as main_block,exp,truncated
from generate_wavpack_float import parts,rewrite,metadata
from generate_wavpack_false_stereo import Bits,log_seed
from generate_wavpack_multichannel import compact
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-correction'
MASK=0xffffffff

def weight(w):
 v=w*8
 return v+((v+64)>>7)if v>0 else v

def predict(state,values,commit):
 term=state['term'];w=state['weights'];h=state['history'];delta=state['delta'];n=len(values);out=values[:]
 def apply(c,s,v):
  result=((w[c]*s+512)>>10)+v
  if commit and s and v:
   w[c]+=delta if (s<0)==(v<0)else-delta
   if term<0:w[c]=max(-1024,min(1024,w[c]))
  return result
 if term>0:
  for c in range(n):
   source=(2*h[c][0]-h[c][1])if term==17 else ((3*h[c][0]-h[c][1])>>1)if term==18 else h[c][0]
   out[c]=apply(c,source,values[c])
   if commit:h[c]=[out[c],h[c][0]]if term>8 else h[c][1:]+[out[c]]
 elif term==-1:
  out[0]=apply(0,h[0][0],values[0]);out[1]=apply(1,out[0],values[1])
  if commit:h[0][0]=out[1]
 elif term==-2:
  out[1]=apply(1,h[1][0],values[1]);out[0]=apply(0,out[1],values[0])
  if commit:h[1][0]=out[0]
 else:
  out[0]=apply(0,h[0][0],values[0]);out[1]=apply(1,h[1][0],values[1])
  if commit:h[0][0]=out[1];h[1][0]=out[0]
 return out

def block(depth,mode,profile,shape,cross,term,index):
 nc=2 if mode=='stereo'else 1
 main,_,trace=main_block(depth,mode,profile,index,entropy_log=8*256)
 med=[[exp(8*256)]*3 for _ in range(nc)];bits=Bits();lossy=[];exact=[]
 for slot,(zone,limit,mid,steps)in enumerate(trace):
  c=slot%nc;low=0
  for k in range(min(zone,2)):low+=(med[c][k]>>4)+1;d=128>>k;med[c][k]+=(med[c][k]+d)//d*5
  k=min(zone,2);band=(med[c][k]>>4)+1
  if zone>2:low+=(zone-2)*band
  high=low+band-1;d=128>>k;med[c][k]+=(med[c][k]+d)//d*5 if zone>2 else -((med[c][k]+d-2)//d*2)
  for branch in range(steps):
   cut=(low+high+1)//2
   if (slot+branch+index)%2:low=cut
   else:high=cut-1
  value=(low if (slot+index)%2 else high)if limit else mid
  if limit:truncated(bits,value-low,high-low)
  sign=(slot//3+index)%2;lossy.append(~mid if sign else mid);exact.append(~value if sign else value)
 delta=2 if depth!=8 else 0;w=[0]*nc if depth==8 else[weight(48),weight(-32)][:nc]
 history=[[1,-2]if term>8 else[1]*max(term,1) for _ in range(nc)]
 if nc==2:history[1]=[-v*2 for v in history[1]]
 taps=([v for h in history for v in h]if term>8 else [history[c][i]for i in range(max(term,1))for c in range(nc)])
 state=dict(term=term,weights=w[:],history=copy.deepcopy(history),delta=delta)
 joint=mode=='stereo'and index%2==1
 error=[2,-2];acc=[0,0];step=[0,0]
 if shape=='compact':acc=[weight(-64)<<16,weight(64)<<16]
 elif shape!='none':acc=[-(1<<25),1<<25];step=[exp(17*256),-exp(16*256)]if shape=='delta'else[0,0]
 shape_body=bytes([192,64])if shape=='compact'else b''.join(struct.pack('<hh',log_seed(error[c]),-26*256 if c==0 else 26*256)for c in range(nc))
 if shape=='compact':error=[0,0]
 if shape=='implicit':error=[0,0];acc=[0,0];step=[0,0]
 if shape=='delta':shape_body+=b''.join(struct.pack('<h',17*256 if c==0 else -16*256)for c in range(nc))
 main_signal=[];full_signal=[]
 for i in range(160):
  l=lossy[i*nc:(i+1)*nc];x=exact[i*nc:(i+1)*nc]
  corrected=predict(copy.deepcopy(state),x,False)if cross and nc==2 else None
  decoded=predict(state,l,True)
  if corrected is None:corrected=[a+b-c for a,b,c in zip(decoded,x,l)]
  if joint:
   for pair in [decoded,corrected]:pair[1]-=pair[0]>>1;pair[0]+=pair[1]
  main_signal+=decoded
  for c in range(nc):
   value=corrected[c]
   if shape!='none':
    acc[c]+=step[c];weight_now=acc[c]>>16;adjustment=-((weight_now*error[c]+512)>>10);offset=value-decoded[c]
    if shape in ['new','delta']and weight_now<0 and adjustment:
     if adjustment==error[c]:adjustment+=1 if adjustment<0 else-1
     error[c]=adjustment-offset
    else:error[c]=-offset
    value-=adjustment
   full_signal.append(value)
 flags=int.from_bytes(main[24:28],'little');flags=(flags&~(31<<18))|(max(max(abs(v)for v in main_signal),max(abs(v)for v in full_signal)).bit_length()<<18)
 if cross:flags|=32
 if joint:flags|=16
 if shape!='none':flags|=64
 if shape in ['new','delta']:flags|=1<<29
 items=[(t,b)for t,b in parts(main)if t not in [2,3,4]]
 weights=bytes([0 if depth==8 else 48]+([0 if depth==8 else 224]if nc==2 else[]))
 items=[(2,bytes([term+5+(delta<<5)])),(3,weights),(4,b''.join(struct.pack('<h',log_seed(v))for v in taps))]+items
 def crc(values):
  c=MASK
  for v in values:c=(c*3+v)&MASK
  return c
 main=rewrite(main,items,flags);main=main[:28]+struct.pack('<I',crc(main_signal))+main[32:]
 body=(metadata(7,shape_body)if shape not in ['none','implicit']else b'')+(metadata(11,bits.close())if bits.bits else b'')
 correction=main[:4]+struct.pack('<I',24+len(body))+main[8:28]+struct.pack('<I',crc(full_signal))+body
 def pcm(values,clip):
  out=[]
  for v in values:
   if clip:v=max(-(1<<(depth-1)),min((1<<(depth-1))-1,v))
   if mode=='false':out.extend([v,v])
   else:out.append(v)
  return b''.join(struct.pack('<f',v/(1<<(depth-1)))for v in out)
 return main,correction,pcm(main_signal,True),pcm(full_signal,False)

def movie(main,correction,channels,depth,add_id=1,duplicate=False,laced=False,lace_kind="xiph",interval_ms=20,duration_ms=60):
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+uint(0x23e383,20_000_000)+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_WAVPACK4')+element(0x63a2,b'\x10\x04')+uint(0x23e383,20_000_000)+element(0x41e4,uint(0x41f0,1)+uint(0x41e7,1))+element(0xe1,element(0xb5,struct.pack('>d',8000))+uint(0x9f,channels)+uint(0x6264,depth))
 clusters=b'';vp9=frame(8,'420',16,16)
 for i,(m,c)in enumerate(zip(main,correction)):
  more=element(0xa6,(uint(0xee,add_id)if i%2 or add_id!=1 or duplicate else b'')+(uint(0xee,add_id)if duplicate else b'')+element(0xa5,c))
  lace=bytes([255])*(len(m)//255)+bytes([len(m)%255]);flag=2
  if lace_kind=='fixed':lace=b'';flag=4
  elif lace_kind=='ebml':
   for width in range(1,9):
    if len(m)<(1<<(7*width))-1:lace=((1<<(7*width))|len(m)).to_bytes(width,'big');break
   flag=6
  payload=element(0xa1,(b'\x82\0\0'+bytes([flag,1])+lace+m+m)if laced else b'\x82\0\0\0'+m);addition=element(0x75a1,more)
  group=element(0xa0,payload+addition if i%2 else addition+payload)
  clusters+=element(0x1f43b675,uint(0xe7,i*interval_ms)+element(0xa3,b'\x81\0\0\x80'+vp9)+group)
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1_000_000)+element(0x4489,struct.pack('>d',duration_ms)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)

def compact_correction(components,channels):
 return b''.join(c[28:32]+((len(c)-32).to_bytes(4,'little')if channels>2 else b'')+c[32:]for c in components)

def generate():
 files={};cases=[]
 for depth,mode,profile,shape,cross in itertools.product([8,16,24,32],['mono','stereo','false'],['snr','bitrate-delta','zero-limit'],['none','compact','old','new','delta','implicit'],[False,True]):
  if mode!='stereo'and cross:continue
  term=([1,8,17,18,-1,-2,-3][len(cases)%7]if mode=='stereo'else[1,8,17,18][len(cases)%4])
  name=f'{depth}-{mode}-{profile}-{shape}-cross{int(cross)}-term{term}';frames=[];full=b'';lossy=b'';mp=[];cp=[]
  for i in range(3):
   m,c,l,x=block(depth,mode,profile,shape,cross,term,i);mn=f'{name}-{i}.wv';cn=f'{name}-{i}.wvc';pn=f'{name}-{i}.packet';an=f'{name}-{i}.additional';files[mn]=m;files[cn]=c;files[pn]=compact([m]);files[an]=compact_correction([c],1 if mode=='mono'else 2);frames.append(dict(file=mn,correction=cn,packet=pn,additional=an,pts=i*160));mp.append(files[pn]);cp.append(files[an]);full+=x;lossy+=l
  files[name+'.f32le']=full;files[name+'-lossy.f32le']=lossy;files[name+'.mkv']=movie(mp,cp,1 if mode=='mono'else 2,depth)
  cases.append(dict(name=name,channels=1 if mode=='mono'else 2,depth=depth,file=name+'.mkv',oracle=name+'.f32le',lossy=name+'-lossy.f32le',frames=frames))
 for profile,shape,cross in itertools.product(['snr','bitrate-delta','zero-limit'],['none','new','delta'],[False,True]):
  depth=16 if len(cases)%2 else 24;name=f'multichannel-{profile}-{shape}-cross{int(cross)}';frames=[];full=b'';lossy=b'';mp=[];cp=[]
  for i in range(3):
   mains=[];corrections=[];controls=[]
   for group,(mode,term)in enumerate([('stereo',-3),('mono',17),('mono',8),('false',18)]):
    m,c,l,x=block(depth,mode,profile,shape,cross,term,i)
    flags=int.from_bytes(m[24:28],'little')&~0x1800
    if group==0:flags|=0x800
    if group==3:flags|=0x1000
    m=rewrite(m,parts(m)+([(13,bytes([6,63,0,0,0]))]if group==0 else[]),flags)
    c=c[:24]+flags.to_bytes(4,'little')+c[28:];mains.append(m);corrections.append(c);controls.append((l,x,1 if mode=='mono'else 2))
   full+=b''.join(x[row*w*4:(row+1)*w*4]for row in range(160)for l,x,w in controls);lossy+=b''.join(l[row*w*4:(row+1)*w*4]for row in range(160)for l,x,w in controls)
   mn=f'{name}-{i}.wv';cn=f'{name}-{i}.wvc';pn=f'{name}-{i}.packet';an=f'{name}-{i}.additional';files[mn]=b''.join(mains);files[cn]=b''.join(corrections);files[pn]=compact(mains);files[an]=compact_correction(corrections,6);mp.append(files[pn]);cp.append(files[an]);frames.append(dict(file=mn,correction=cn,packet=pn,additional=an,pts=i*160))
  files[name+'.f32le']=full;files[name+'-lossy.f32le']=lossy;files[name+'.mkv']=movie(mp,cp,6,depth)
  cases.append(dict(name=name,channels=6,depth=depth,file=name+'.mkv',oracle=name+'.f32le',lossy=name+'-lossy.f32le',frames=frames))
 base=cases[0];first=base['frames'][0];m=files[first['file']];c=files[first['correction']];invalid=[]
 for name,bad,message in [
  ('crc',c[:28]+bytes([c[28]^1])+c[29:],'checksum does not match'),
  ('missing-bits',rewrite(c,[(t,b)for t,b in parts(c)if t!=11]),'ends inside'),
  ('extra-word',rewrite(c,[(t,b+b'\xff\xff'if t==11 else b)for t,b in parts(c)]),'leaves its extra bits unread')]:
  fn=name+'.wvc';mv=name+'.mkv';files[fn]=bad;files[mv]=movie([files[first['packet']]],[compact_correction([bad],1)],1,8);invalid.append(dict(file=fn,movie=mv,message=message))
 files['mismatch-window.wvc']=c[:16]+(1).to_bytes(4,'little')+c[20:]
 files['compact-short.mkv']=movie([files[first['packet']]],[bytes(3)],1,8)
 files['duplicate-id.mkv']=movie([files[first['packet']]],[files[first['additional']]],1,8,duplicate=True)
 files['laced-correction.mkv']=movie([files[first['packet']]],[files[first['additional']]],1,8,laced=True)
 files['ignored-id.mkv']=movie([files[f['packet']]for f in base['frames']],[files[f['additional']]for f in base['frames']],1,8,add_id=2)
 atomic=next(c for c in cases if c['name']=='multichannel-snr-none-cross0');f=atomic['frames'][0]
 def components(raw):
  result=[];at=0
  while at<len(raw):length=int.from_bytes(raw[at+4:at+8],'little')+8;result.append(raw[at:at+length]);at+=length
  return result
 ms=components(files[f['file']]);cs=components(files[f['correction']]);ms[0]=rewrite(ms[0],[(t,bytes([6])+(0x60f).to_bytes(4,'little')if t==13 else b)for t,b in parts(ms[0])]);last=cs[-1];cs[-1]=last[:28]+bytes([last[28]^1])+last[29:]
 files['atomic-mask.wv']=b''.join(ms);files['atomic-last-crc.wvc']=b''.join(cs);files['atomic-mask.mkv']=movie([compact(ms)],[compact_correction(cs,6)],6,atomic['depth'])
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=invalid,atomic_case=atomic['name']),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
