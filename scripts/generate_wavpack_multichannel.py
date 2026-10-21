"""Owned complete WavPack sample frames and standard Matroska multichannel mapping.
No external codec/process/network, private input or generated artifact is read.
"""
import json,struct,itertools
from pathlib import Path
from generate_wavpack_false_stereo import block as integer_block
from generate_wavpack_float import block as float_block,integer_block as optimized_block,parts,rewrite,metadata
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-multichannel'
def layout(channels,kind):
 if kind=='mono':return [1]*channels
 if channels==6:return [2,1,1,2]
 if channels==8:return [2,1,1,2,2]
 return [2]*(channels//2)+([1]if channels%2 else[])
def channel_info(channels,streams,mask,new,short=False):
 if not new:return bytes([channels])+mask.to_bytes((mask.bit_length()+7)//8 if short else 4,'little')
 c=channels-1;s=streams-1
 return bytes([c&255,s&255,((c>>8)&15)|((s>>4)&240)])+mask.to_bytes(3 if channels in [3,6] else 4,'little')
def compact(blocks):
 if len(blocks)==1 and (int.from_bytes(blocks[0][24:28],'little')&0x1800)==0x1800:return blocks[0][20:]
 return blocks[0][20:24]+b''.join(p[24:32]+(len(p)-32).to_bytes(4,'little')+p[32:]for p in blocks)
def movie(packets,rate,channels,depth):
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+uint(0x23e383,20_000_000)+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_WAVPACK4')+element(0x63a2,b'\x10\x04')+uint(0x23e383,20_000_000)+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels)+uint(0x6264,depth))
 clusters=b'';vp9=frame(8,'420',16,16)
 for i,p in enumerate(packets):clusters+=element(0x1f43b675,uint(0xe7,i*20)+element(0xa3,b'\x81\0\0\x80'+vp9)+element(0xa3,b'\x82\0\0\x80'+p))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1_000_000)+element(0x4489,struct.pack('>d',60)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)
def generate():
 files={};cases=[]
 def emit(channels,kind,coding,new,wide=False,short=False,unassigned=False):
  name=f'c{channels}-{kind}-{coding}-new{int(new)}'+('-wide'if wide else'')+('-short'if short else'')+('-unassigned'if unassigned else'');rate=[8000,48000,37500][len(cases)%3];n=rate//50;groups=layout(channels,kind)
  mask=0 if unassigned else {3:7,6:63,8:0x63f}.get(channels,0)
  frames=[];packets=[];oracle=b''
  for i in range(3):
   blocks=[];lanes=[];controls=[]
   for group,width in enumerate(groups):
    mode='mono'if width==1 else 'false'if kind=='false'else'stereo'
    if coding=='float':raw,pcm,_=float_block('false'if mode=='mono'else mode,rate,i,127+group%6,0,'optimized')
    elif coding=='optimized':raw,pcm=optimized_block('false'if mode=='mono'else mode,rate,i,group%2!=0)
    else:raw,pcm=integer_block(int(coding),1,group>=4,rate,i*n,mode,'none'if coding!='32'else['none','sent','sent-dups'][group%3],amplitude=1<<(group%4))
    if mode=='mono'and coding in ['float','optimized']:
     f=(int.from_bytes(raw[24:28],'little')&~(1<<30))|4;raw=rewrite(raw,parts(raw),f);pcm=b''.join(pcm[j:j+4]for j in range(0,len(pcm),8))
    control=f'{name}-{i}-component{group}.wv';files[control]=raw;controls.append(dict(file=control,channels=width))
    flags=int.from_bytes(raw[24:28],'little')&~0x1800
    if group==0:flags|=0x800
    if group==len(groups)-1:flags|=0x1000
    items=parts(raw)
    if group==0:items.insert(0,(13,channel_info(channels,len(groups),mask,new,short)))
    raw=bytearray(rewrite(raw,items,flags))
    if wide:
     raw[10]=1;raw[11]=1;raw[12:16]=(n*3+1).to_bytes(4,'little')
    blocks.append(bytes(raw));lanes.append(pcm)
   output=bytearray()
   for row in range(n):
    for lane,width in zip(lanes,groups):output+=lane[row*width*4:(row+1)*width*4]
   fn=f'{name}-{i}.wv';pn=f'{name}-{i}.packet';files[fn]=b''.join(blocks);files[pn]=compact(blocks);packets.append(files[pn]);oracle+=output
   frames.append(dict(file=fn,packet=pn,components=controls,pts=(1<<32 if wide else 0)+i*n))
  depth=32 if coding in ['float','optimized']else int(coding)
  files[name+'.mkv']=movie(packets,rate,channels,depth);files[name+'.f32le']=oracle
  cases.append(dict(name=name,channels=channels,mask=mask,rate=rate,depth=depth,file=name+'.mkv',oracle=name+'.f32le',frames=frames))
 for channels,kind,coding,new in itertools.product([3,6,8],['paired','mono','false'],['8','16','24','32','float','optimized'],[False,True]):emit(channels,kind,coding,new)
 emit(3,'paired','16',False,short=True)
 emit(8,'paired','32',False,short=True)
 emit(6,'mono','float',False,short=True,unassigned=True)
 for channels in [20,256,4096]:emit(channels,'paired','16',True)
 emit(6,'paired','16',True,True)
 # Frame-level malformed vectors retain valid per-component coding parameters.
 base=next(c for c in cases if c['name']=='c6-paired-16-new1');raw=files[base['frames'][0]['file']]
 blocks=[];at=0
 while at<len(raw):size=int.from_bytes(raw[at+4:at+8],'little')+8;blocks.append(raw[at:at+size]);at+=size
 bad=[]
 def mutate(name,group,offset,value,message):
  altered=blocks[:];p=bytearray(altered[group]);p[offset:offset+4]=value.to_bytes(4,'little');altered[group]=bytes(p);record(name,altered,message)
 def record(name,altered,message):
  fn=name+'.wv';pn=name+'.packet';mv=name+'.mkv';files[fn]=b''.join(altered);files[pn]=compact(altered);files[mv]=movie([files[pn]]+[files[f['packet']]for f in base['frames'][1:]],base['rate'],6,16);bad.append(dict(file=fn,packet=pn,movie=mv,message=message,raw_only=name in ['different-window','different-count','different-wide-index','component-size-overflow','different-total']))
 flags=lambda group:int.from_bytes(blocks[group][24:28],'little')
 mutate('missing-initial',0,24,flags(0)&~0x800,'initial block sequence')
 mutate('repeated-initial',1,24,flags(1)|0x800,'initial block sequence')
 mutate('missing-final',len(blocks)-1,24,flags(len(blocks)-1)&~0x1000,'missing final block')
 mutate('early-final',0,24,flags(0)|0x1000,'trailing bytes after final')
 mutate('component-size-overflow',1,4,0xfffffff0,'block runs past the packet')
 mutate('different-window',1,16,1,'sample window mismatch')
 mutate('different-count',1,20,base['rate']//50-1,'sample window mismatch')
 mutate('different-total',1,12,base['rate']//50*3+1,'total sample count mismatch')
 mutate('different-rate',1,24,(flags(1)&~(15<<23))|(2<<23),'format mismatch')
 mutate('last-crc',len(blocks)-1,28,0,'checksum')
 altered=blocks[:];p=bytearray(altered[1]);p[10]=1;altered[1]=bytes(p);record('different-wide-index',altered,'sample window mismatch')
 info=parts(blocks[0])[0][1];wrong=bytes([info[0]+1])+info[1:];altered=blocks[:];altered[0]=rewrite(blocks[0],[(13,wrong)]+parts(blocks[0])[1:]);record('wrong-channel-info',altered,'channel information disagrees')
 altered=blocks[:];altered[0]=rewrite(blocks[0],[(13,info[:2])]+parts(blocks[0])[1:]);record('wrong-legacy-info',altered,'channel information disagrees')
 changedmask=bytearray(info);changedmask[3:7]=(0x60f).to_bytes(4,'little');altered=blocks[:];altered[1]=rewrite(blocks[1],[(13,bytes(changedmask))]+parts(blocks[1]));record('conflicting-mask',altered,'channel information disagrees')
 # A later well-formed frame may not silently change the stream's layout.
 changed=files[base['frames'][1]['file']];grouped=[];at=0
 while at<len(changed):length=int.from_bytes(changed[at+4:at+8],'little')+8;grouped.append(changed[at:at+length]);at+=length
 first=parts(grouped[0]);newmask=bytearray(first[0][1]);newmask[3:7]=(0x60f).to_bytes(4,'little');grouped[0]=rewrite(grouped[0],[(13,bytes(newmask))]+first[1:])
 files['layout-change.wv']=b''.join(grouped);files['layout-change.packet']=compact(grouped)
 corrupt=grouped[:];last=bytearray(corrupt[-1]);last[28:32]=bytes(4);corrupt[-1]=bytes(last)
 files['uncommitted-mask.wv']=b''.join(corrupt)
 files['uncommitted-mask.mkv']=movie([compact(corrupt)]+[files[f['packet']]for f in base['frames'][1:]],base['rate'],6,16)
 files['layout-change.mkv']=movie([files[base['frames'][0]['packet']],files['layout-change.packet'],files[base['frames'][2]['packet']]],base['rate'],6,16)
 mapping=[];original=files[base['frames'][0]['packet']]
 for name,payload,message in [
  ('compact-short',original[:8],'truncated Matroska'),
  ('compact-overrun',original[:12]+(0xffffffff).to_bytes(4,'little')+original[16:],'component exceeds packet'),
  ('compact-tail',original+b'\0','component header')]:
  fn=name+'.packet';mv=name+'.mkv';files[fn]=payload;files[mv]=movie([payload]+[files[f['packet']]for f in base['frames'][1:]],base['rate'],6,16);mapping.append(dict(file=fn,movie=mv,message=message))
 # Version is the shared CodecPrivate field, not bytes retained in compact blocks.
 future=movie([files[f['packet']]for f in base['frames']],base['rate'],6,16).replace(element(0x63a2,b'\x10\x04'),element(0x63a2,b'\x11\x04'),1)
 files['future-version.mkv']=future
 mapping.append(dict(file=base['frames'][0]['packet'],movie='future-version.mkv',private=[0x11,4],message='invalid Matroska WavPack version'))
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=bad,mapping_invalid=mapping),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
