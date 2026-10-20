#!/usr/bin/env python3
"""Offline authored Xing/LAME gapless tags and owned compressed audio companions.
No encoder, FFmpeg, network or private parameters are used.
"""
from pathlib import Path
import json,struct,hashlib
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
R=Path(__file__).resolve().parents[1];S=R/'tests/fixtures/playback-errors/mp3-decoder';D=S.parent/'mp3-gapless';D.mkdir(exist_ok=True)
def crc(data,reflected):
 c=0 if reflected else 65535
 for v in data:
  if reflected:
   c^=v
   for _ in range(8):c=(c>>1)^(0xa001 if c&1 else 0)
  else:
   for i in range(7,-1,-1):
    feedback=(c>>15)^((v>>i)&1);c=(c<<1)&65535
    if feedback:c^=0x8005
 return c
def info(template,rate,channels,flags,protected,aware,delay=7,padding=600,style='standard'):
 header=bytearray(template[:4]);header[1]=(header[1]&254)|int(not protected)
 out=bytearray(len(template));out[:4]=header
 side=(17 if channels==1 else 32)if rate>=32000 else(9 if channels==1 else 17)
 off=4+side+(2 if protected and aware else 0)
 values=b'Info'+struct.pack('>I',flags)
 for bit,payload in [(1,struct.pack('>I',4)),(2,struct.pack('>I',len(template)*5+2)),(4,bytes(range(100))),(8,struct.pack('>I',42))]:
  if flags&bit:values+=payload
 out[off:off+len(values)]=values;at=off+len(values)
 tag=bytearray(36);tag[:9]=b'LAMEtest!';tag[21:24]=((delay<<12)|padding).to_bytes(3,'big');out[at:at+36]=tag
 if protected:out[4:6]=crc(out[2:4]+out[6:6+side],False).to_bytes(2,'big')
 if style=='fixed190':
  out[at:at+9]=b'LavcTest!';value=crc(out[:190],True)
 else:value=crc(out[:at+34],True)
 out[at+34:at+36]=value.to_bytes(2,'big')
 return bytes(out),off,at
def movie(packets,rate,channels,head,tail):
 ns=lambda n:n*1000000000//rate
 samples=1152 if rate>=32000 else 576
 header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 trackv=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',ns(samples).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 tracka=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_MPEG/L3')+ebml('56aa',ns(head).to_bytes(4,'big'))+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
 blocks=ebml('e7',b'\0')
 for i,pkt in enumerate(packets):
  stamp=round(i*samples*1000/rate)
  blocks+=ebml('a3',b'\x81'+struct.pack('>h',stamp)+b'\x80'+picture(8,'420',16,16))
  group=ebml('a1',b'\x82'+struct.pack('>h',stamp)+b'\0'+pkt)
  if i==len(packets)-1 and tail:group+=ebml('75a2',ns(tail).to_bytes(8,'big',signed=True))
  blocks+=ebml('a0',group)
 return header+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',trackv+tracka)+ebml('1f43b675',blocks))
def free_packet(packet,rate,channels):
 out=bytearray(packet);out[2]&=15
 side=(17 if channels==1 else 32)if rate>=32000 else(9 if channels==1 else 17)
 if out[1]&1==0:out[4:6]=crc(out[2:4]+out[6:6+side],False).to_bytes(2,'big')
 return bytes(out)
cases=[];files={};vectors=[]
for source in json.loads((S/'manifest.json').read_text())['cases']:
 name=source['name'];rate=source['rate'];channels=source['channels'];data=(S/(name+'.mp3')).read_bytes();sizes=source['sizes'];first=data[:sizes[0]];coded=source['samples'];begin=536;end=coded-71
 for protected,aware in [(False,False),(True,False),(True,True)]:
  for flags in range(16):
   raw,off,at=info(first,rate,channels,flags,protected,aware)
   vectors.append(struct.pack('>I4BHHI',len(raw),flags,protected,aware,1,7,600,off)+raw)
  label=f'{name}-'+('plain'if not protected else'aware'if aware else'legacy')
  raw,off,at=info(first,rate,channels,15,protected,aware)
  files[label+'.mp3']=raw+data
  packets=[];pos=0
  for size in sizes:packets.append(data[pos:pos+size]);pos+=size
  files[label+'.mkv']=movie(packets,rate,channels,begin,coded-end)
  free=list(map(lambda p:free_packet(p,rate,channels),packets))
  files[label+'-free.mp3']=raw+b''.join(free)
  files[label+'-free.mkv']=movie(free,rate,channels,begin,coded-end)
  cases.append({'name':label+'-free','source':name,'rate':rate,'channels':channels,'coded':coded,'start':begin,'end':end})
  cases.append({'name':label,'source':name,'rate':rate,'channels':channels,'coded':coded,'start':begin,'end':end})
 # Bad metadata must preserve the original coded PCM rather than trimming it.
 if name=='48000-1ch':
  for label,delay,padding,corrupt,revision,count in [('bad-crc',7,600,True,0,4),('unknown-revision',7,600,False,1,4),('impossible-window',4095,4095,False,0,4),('wrong-frame-count',7,600,False,0,5)]:
   raw,off,at=info(first,rate,channels,15,False,False,delay,padding);raw=bytearray(raw)
   raw[at+9]=revision<<4;raw[off+8:off+12]=count.to_bytes(4,'big');raw[at+34:at+36]=crc(raw[:at+34],True).to_bytes(2,'big')
   if corrupt:raw[at+21]^=1
   files[label+'.mp3']=bytes(raw)+data
   cases.append({'name':label,'source':name,'rate':rate,'channels':channels,'coded':coded,'start':0,'end':coded,'no_video':True})
  raw,off,at=info(first,rate,channels,15,False,False,style='fixed190');files['fixed190.mp3']=raw+data
  cases.append({'name':'fixed190','source':name,'rate':rate,'channels':channels,'coded':coded,'start':begin,'end':end,'no_video':True})
# A valid first audio frame borrows sixteen coded bytes from the Info frame.
base=next(c for c in cases if c['name']=='48000-1ch-aware')
source=(S/'48000-1ch.mp3').read_bytes();sizes=[960,961,960,961];head=bytearray(files['48000-1ch-aware.mp3'][:960]);audio=[];pos=0
for size in sizes:audio.append(bytearray(source[pos:pos+size]));pos+=size
main=23;head[-16:]=audio[0][main:main+16]
audio[0][6]=8;audio[0][7]&=127 # main_data_begin = 16.
audio[0][main:]=audio[0][main+16:]+bytes(16)
audio[0][4:6]=crc(audio[0][2:4]+audio[0][6:23],False).to_bytes(2,'big')
files['primed-history.mp3']=bytes(head)+b''.join(audio)
packets=[bytes(head)]+list(map(bytes,audio));rate=48000
files['primed-history.mkv']=movie(packets,rate,1,1688,71)
cases.append(dict(base,name='primed-history',video_frames=5))
free=list(map(lambda p:free_packet(p,48000,1),audio))
files['primed-free.mp3']=bytes(head)+b''.join(free)
free_head=bytearray(free_packet(head,48000,1));offset=23+120
free_head[offset+34:offset+36]=crc(free_head[:offset+34],True).to_bytes(2,'big')
files['primed-free.mkv']=movie([bytes(free_head)]+free,48000,1,1688,71)
cases.append(dict(base,name='primed-free',video_frames=5))
# A coded audio frame whose first Huffman bytes happen to spell Info is audio.
bits=[0]*18
def put(value,n):bits.extend((value>>i)&1 for i in range(n-1,-1,-1))
for _ in range(2):
 for value,n in [(2000,12),(288,9),(140,8),(0,4),(0,1),(1,5),(1,5),(1,5),(7,4),(5,3),(0,1),(0,1),(0,1)]:put(value,n)
side=bytes(sum(bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(bits),8))
coded=bytearray(source[:960]);coded[6:23]=side;coded[23:]=bytes(937);coded[23:27]=b'Info'
coded[4:6]=crc(coded[2:4]+side,False).to_bytes(2,'big')
# Independent PCM oracle: subtract the authored original first-frame spectra
# and add the marker frame spectra on the absolute synthesis timeline. All
# later start/short/stop mixed-window contributions remain in the source oracle.
from mp3_synthesis_oracle import oracle
book=json.loads((D.parent/'mp3-huffman/codewords.json').read_text())['books']
entries=next(b['entries'] for b in book if b['id']==1)
lookup={(length,word):symbol for word,length,symbol in entries}
main_bits=''.join(f'{v:08b}' for v in coded[23:])
deltas=[]
for granule in range(2):
 pos=granule*2000;q=[]
 for pair in range(288):
  word=0
  for length in range(1,20):
   word=(word<<1)|int(main_bits[pos]);pos+=1
   if (length,word) in lookup:break
  else:raise AssertionError('invalid authored Huffman word')
  symbol=lookup[length,word]
  for value in [symbol>>4,symbol&15]:
   if value:
    value*=1-2*int(main_bits[pos]);pos+=1
   q.append(value)
 old=[0.]*576
 for i in [0,34,36,72,108,574]:old[i]=(1 if (i+granule)%2==0 else -1)*2**(-7.5)
 deltas.append((0,0,[value*2**(-17.5)-before for value,before in zip(q,old)]))
deltas.extend((0,0,[0.]*576) for _ in range(6))
original=struct.unpack('<4608f',(S/'48000-1ch.f32le').read_bytes())
files['coded-marker.f32le']=b''.join(struct.pack('<f',value+delta) for value,delta in zip(original,oracle(deltas)))
rest=source[960:];files['coded-marker.mp3']=bytes(coded)+rest
frames=[bytes(coded),source[960:1921],source[1921:2881],source[2881:]]
files['coded-marker.mkv']=movie(frames,48000,1,0,0)
files['info-only.mp3']=bytes(head)
for name,data in files.items():(D/name).write_bytes(data)
raw=b'FVMP3G1\0'+struct.pack('>I',len(vectors))+b''.join(vectors);(D/'tags.bin').write_bytes(raw);files['tags.bin']=raw
(D/'manifest.json').write_text(json.dumps({'cases':cases,'tag_vectors':len(vectors),'provenance':'Authored metadata and FVid-authored audio, independent source PCM windows, neutral VP9 companions','files':{name:hashlib.sha256(data).hexdigest()for name,data in files.items()}},indent=2)+'\n')
print(len(cases),'gapless cases and',len(vectors),'tag field vectors')
