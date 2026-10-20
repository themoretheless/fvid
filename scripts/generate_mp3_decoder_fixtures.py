#!/usr/bin/env python3
"""Offline authored compressed Layer III chains and neutral video companions.
Own bit packing and absolute-timeline/convolution PCM oracle, no external codec.
"""
import json,struct,hashlib,math,bisect
from pathlib import Path
from mp3_synthesis_oracle import oracle
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
R=Path(__file__).resolve().parents[1];D=R/'tests/fixtures/playback-errors/mp3-decoder';D.mkdir(exist_ok=True)
B=json.loads((D.parent/'mp3-spectral/bands.json').read_text())['rates']
books={b['id']:{entry[2]:(entry[0],entry[1])for entry in b['entries']}for b in json.loads((D.parent/'mp3-huffman/codewords.json').read_text())['books']}
class Bits:
 def __init__(self):self.bits=[]
 def put(self,v,n):self.bits.extend((v>>i)&1 for i in range(n-1,-1,-1))
 def packed(self):
  values=self.bits+[0]*(-len(self.bits)%8)
  return bytes(sum(values[i+j]<<(7-j)for j in range(8))for i in range(0,len(values),8))
def crc(data):
 value=65535
 for byte in data:
  for shift in range(7,-1,-1):
   feedback=(value>>15)^((byte>>shift)&1);value=(value<<1)&65535
   if feedback:value^=0x8005
 return value
cases=[];files={}
for rate in [48000,24000,8000,44100,22050,11025,32000,16000,12000]:
 version=3 if rate>=32000 else 2 if rate>=16000 else 0;mpeg1=version==3
 lb,sb=B[str(rate)]['long'],B[str(rate)]['short'];ng=2 if mpeg1 else 1
 for channels in [1,2]:
  frames=[];granules=[[],[]];sizes=[]
  for number,kind in enumerate([0,1,2,3]):
   mixed=kind!=0;mode=3 if channels==1 else 1;extension=0 if channels==1 else 2
   pad=number%2;bitrate=320000 if mpeg1 else 160000;rate_index=2 if rate in [32000,16000,8000] else 0 if rate in [44100,22050,11025] else 1
   header=((2047<<21)|(version<<19)|(1<<17)|(14<<12)|(rate_index<<10)|(pad<<9)|(mode<<6)|(extension<<4)).to_bytes(4,'big')
   side=Bits();side.put(0,9 if mpeg1 else 8);side.put(0,5 if mpeg1 and channels==1 else 3 if mpeg1 else 1 if channels==1 else 2)
   if mpeg1:
    for ch in range(channels):side.put(0,4)
   main=Bits()
   for g in range(ng):
    spectra=[]
    for ch in range(channels):
     q=[0]*576
     for i in [ch,34+ch,36+ch,72+ch,108+ch,574+ch]:q[i]=1 if (i+g+number)%2==0 else -1
     first=lb[8]if kind==0 else 72 if kind==2 and rate==8000 else 36 if kind==2 or mpeg1 else 108 if rate==8000 else 54
     second=lb[14]if kind==0 else 576
     word=Bits()
     for i in range(0,576,2):
      table=[1,2,3][0 if i<first else 1 if i<second else 2]
      code,length=books[table][16*abs(q[i])+abs(q[i+1])];word.put(code,length)
      for v in q[i:i+2]:
       if v:word.put(int(v<0),1)
     for v,n in [(len(word.bits),12),(288,9),(180,8),(0,4 if mpeg1 else 9),(int(kind!=0),1)]:side.put(v,n)
     if kind:
      side.put(kind,2);side.put(1,1);side.put(1,5);side.put(2,5)
      for w in range(3):side.put(0,3)
     else:
      for table in [1,2,3]:side.put(table,5)
      side.put(7,4);side.put(5,3)
     if mpeg1:side.put(0,1)
     side.put(0,1);side.put(0,1);main.bits.extend(word.bits)
     # Inverse per-output mapping, independent of production traversal.
     nlong=(8 if mpeg1 else 6)if kind==2 else 22;boundary=lb[nlong]
     xr=[]
     for out in range(576):
      if out<boundary:source=out
      else:
       frequency,w=divmod(out,3);band=bisect.bisect_right(sb,frequency)-1
       source=3*sb[band]+w*(sb[band+1]-sb[band])+frequency-sb[band]
      xr.append(q[source]*2**(-7.5))
     spectra.append(xr)
    if channels==2:spectra=[[(a+b)/math.sqrt(2)for a,b in zip(*spectra)],[(a-b)/math.sqrt(2)for a,b in zip(*spectra)]]
    for ch in range(channels):granules[ch].append((kind,4 if rate==8000 and mixed else 2 if mixed else 0,spectra[ch]))
   rawside=side.packed();assert len(rawside)==(17 if channels==1 else 32)if mpeg1 else len(rawside)==(9 if channels==1 else 17)
   payload=header+struct.pack('>H',crc(header[2:]+rawside))+rawside+main.packed()
   length=(144 if mpeg1 else 72)*bitrate//rate+pad;assert len(payload)<=length
   payload+=bytes(length-len(payload));frames.append(payload);sizes.append(length)
  name=f'{rate}-{channels}ch';raw=b''.join(frames);files[name+'.mp3']=raw
  pcm=[oracle(granules[ch])for ch in range(channels)]
  files[name+'.f32le']=b''.join(struct.pack('<f',pcm[ch][i])for i in range(len(pcm[0]))for ch in range(channels))
  header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
  info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
  video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',round(ng*576*1e9/rate).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
  audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_MPEG/L3')+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
  blocks=ebml('e7',b'\0')
  for i,packet in enumerate(frames):
   time=round(i*ng*576*1000/rate);blocks+=ebml('a3',b'\x81'+struct.pack('>h',time)+b'\x80'+picture(8,'420',16,16))
   blocks+=ebml('a3',b'\x82'+struct.pack('>h',time)+b'\x80'+packet)
  files[name+'.mkv']=header+ebml('18538067',info+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
  cases.append({'name':name,'rate':rate,'channels':channels,'sizes':sizes,'samples':len(pcm[0])})
for name,raw in files.items():(D/name).write_bytes(raw)
(D/'manifest.json').write_text(json.dumps({'scope':'Authored compressed Layer III chain PCM acceptance; protected frames, long/start/short/stop mixed windows and MS stereo','cases':cases,'files':{name:hashlib.sha256(raw).hexdigest()for name,raw in files.items()}},indent=2)+'\n')
print(len(cases),'compressed MP3 chains with video companions and independent PCM')
