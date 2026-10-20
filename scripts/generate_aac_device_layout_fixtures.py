#!/usr/bin/env python3
"""Own AAC PCE layouts in Matroska/MP4 videos. Offline explicit generation."""
import json,hashlib,struct,math
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
from generate_he_aac_packet_fixtures import video_fixture,boxes,box
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';OUT=ROOT/'aac-device-layout'
def packed(fields):
 bits=''.join(f'{v:0{n}b}'for v,n in fields);bits+='0'*(-len(bits)%8);return int(bits,2).to_bytes(len(bits)//8,'big')
def generate():
 files={};cases=[];delays=[]
 for name,c in json.loads((ROOT/'aac-height-generated.json').read_text())['cases'].items():
  if 'elements'not in c:continue
  raw=(ROOT/f'aac-height-{name}.aac').read_bytes();packets=[];at=0
  while at<len(raw):
   h=raw[at:at+7];n=((h[3]&3)<<11)|(h[4]<<3)|(h[5]>>5);assert n>=7;packets.append(raw[at+7:at+n]);at+=n
  assert at==len(raw)
  elements=c['elements'];channels=len(c['order']);fields=[(0,4),(1,2),(3,4)]+[(sum(p==pos for p,*_ in elements),width)for pos,width in [('front',4),('side',4),('back',4),('lfe',2)]]+[(0,3),(0,4),(0,1),(0,1),(0,1)]
  for pos,pair,tag,height in elements:
   if pos!='lfe':fields.append((int(pair),1))
   fields.append((tag,4))
  body=sum(n for _,n in fields);comment_at=(3+body+7)//8;size=packets[0][comment_at];comment=packets[0][comment_at+1:comment_at+1+size];assert len(comment)==size
  prefix=[(2,5),(3,4),(0,4),(0,1),(0,1),(0,1)]+fields;used=sum(n for _,n in prefix)
  if used%8:prefix.append((0,8-used%8))
  asc=packed(prefix+[(len(comment),8)]+[(v,8)for v in comment]);files[name+'.asc']=asc
  video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(1024*1000000000//48000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
  def audio_track(delay=0):
   return ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_AAC')+ebml('63a2',asc)+(ebml('56aa',delay.to_bytes(8,'big'))if delay else b'')+ebml('e1',ebml('b5',struct.pack('>d',48000))+ebml('9f',bytes([channels]))))
  audio=audio_track()

  clusters=b''
  for i,p in enumerate(packets):
   blocks=ebml('e7',(i*1024*1000000000//48000).to_bytes(8,'big'))+ebml('a3',b'\x81\0\0\x80'+picture(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+p);clusters+=ebml('1f43b675',blocks)
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',b'\1'))+ebml('1654ae6b',video+audio)+clusters)
  if name=='normal-ten':
   for label,delay in [('floor',17*1000000000//48000),('ceil',(17*1000000000+47999)//48000)]:
    filename=name+'-delay-'+label+'.mkv'
    files[filename]=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',b'\1'))+ebml('1654ae6b',video+audio_track(delay))+clusters)
    delays.append(dict(file=filename,base=name,head=17,delay_ns=delay,samples=6144-17))
  frames=[];blob=b''
  for p in packets:frames.append(dict(offset=len(blob),bytes=len(p)));blob+=p
  mp4=video_fixture([dict(slots=16,bands=64,asc=asc.hex(),frames=frames,container_frame_samples=1024,container_rate=48000,pcm_offset=0,samples=1024*len(packets))],blob,channels,filename=f'{name}.mp4',write=False)
  k=math.sqrt(0.5);C=[k,k];L=[k,0.];R=[0.,k];Z=[0.,0.]
  authored={
   'mono-top':[C],'mono-bottom':[C],'layered-front':[C,C,C],'layered-front-swap':[C,C,C],
   'top-front-pair':[C,L,R],'top-front-three':[L,C,R],'top-back-three':[L,C,R],
   'top-side':[C,L,R],'bottom-surround':[Z,C,L,R,L,R],'top-wide':[C,L,R,L,R],
   'top-with-lfe':[C,Z,C,C],'normal-four':[L,R,[1.,0.],[0.,1.]],
   'multiple-lfe':[C,Z,Z],'normal-three-sce':[C,[1.,0.],[0.,1.]],
   'normal-ten':[L,R,L,R,L,R,L,R,[1.,0.],[0.,1.]],
   'normal-crc':[[1.,1.]],'short-comment':[[1.,1.]],'short-comment-two':[[1.,1.]],
  }[name];assert len(authored)==channels
  source=(ROOT/f'aac-height-{name}.f32le').read_bytes();pcm=struct.unpack('<'+'f'*(len(source)//4),source);stereo=bytearray()
  for i in range(0,len(pcm),channels):
   stereo+=struct.pack('<ff',*(sum(pcm[i+j]*authored[j][side]for j in range(channels))for side in range(2)))
  files[name+'.stereo.f32le']=bytes(stereo)
  files[name+'.mp4']=mp4['data'];cases.append(dict(name=name,channels=channels,samples=1024*len(packets),order=c['order'],elements=elements,weights=authored))
 edits=[]
 for base in ['layered-front','normal-ten']:
  original=files[base+'.mp4'];outer=list(boxes(original));movie=next(body for tag,body in outer if tag==b'moov');rate=struct.unpack_from('>I',next(body for tag,body in boxes(movie)if tag==b'mvhd'),12)[0];
  original_rate=rate;rate=48000;duration=6096
  def rewrite(tag,body):
   if tag==b'mvhd':
    body=bytearray(body);old=struct.unpack_from('>I',body,16)[0];struct.pack_into('>II',body,12,rate,old*rate//original_rate);return box(tag,bytes(body))
   if tag==b'tkhd':
    body=bytearray(body);old=struct.unpack_from('>I',body,20)[0];struct.pack_into('>I',body,20,old*rate//original_rate);return box(tag,bytes(body))
   if tag==b'trak':
    mdia=next((p for t,p in boxes(body)if t==b'mdia'),b'');handler=next((p[8:12]for t,p in boxes(mdia)if t==b'hdlr'),b'')
    if handler==b'soun':
     children=[]
     for t,p in boxes(body):
      if t==b'edts':continue
      if t==b'tkhd':p=bytearray(p);struct.pack_into('>I',p,20,duration);p=bytes(p)
      children.append(box(t,p))
     children.append(box(b'edts',box(b'elst',bytes(4)+struct.pack('>IIiHH',1,duration,48,1,0))))
     return box(tag,b''.join(children))
   if tag in [b'moov',b'trak']:body=b''.join(rewrite(t,p)for t,p in boxes(body))
   return box(tag,body)
  name=base+'-priming.mp4';files[name]=b''.join(rewrite(t,p)for t,p in outer);edits.append(dict(file=name,base=base,head=48,samples=6096))
 files['manifest.json']=(json.dumps(dict(cases=cases,edits=edits,delays=delays,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 files=generate()
 for n,v in files.items():(OUT/n).write_bytes(v)
 print(len(json.loads(files['manifest.json'])['cases']),'authored AAC device layout cases')
