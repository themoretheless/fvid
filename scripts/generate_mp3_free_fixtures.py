#!/usr/bin/env python3
"""Offline authored free-format MP3 frames, false sync words and video companions.
Starts from saved FVid-authored chains; preserves their independent PCM oracle.
"""
import json,struct,hashlib
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
R=Path(__file__).resolve().parents[1];S=R/'tests/fixtures/playback-errors/mp3-decoder';D=S.parent/'mp3-free';D.mkdir(exist_ok=True)
def crc(data):
 value=65535
 for byte in data:
  for shift in range(7,-1,-1):
   feedback=(value>>15)^((byte>>shift)&1);value=(value<<1)&65535
   if feedback:value^=0x8005
 return value
def movie(frames,rate,channels,samples):
 header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',round(samples*1e9/rate).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_MPEG/L3')+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
 blocks=ebml('e7',b'\0')
 for i,packet in enumerate(frames):
  time=round(i*samples*1000/rate);blocks+=ebml('a3',b'\x81'+struct.pack('>h',time)+b'\x80'+picture(8,'420',16,16))
  blocks+=ebml('a3',b'\x82'+struct.pack('>h',time)+b'\x80'+packet)
 return header+ebml('18538067',info+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
cases=[];files={}
for source in json.loads((S/'manifest.json').read_text())['cases']:
 name=source['name'];rate=source['rate'];channels=source['channels'];mpeg1=rate>=32000;samples=1152 if mpeg1 else 576
 raw=(S/(name+'.mp3')).read_bytes();at=0;original=[]
 for size in source['sizes']:original.append(raw[at:at+size]);at+=size
 side=(17 if channels==1 else 32)if mpeg1 else(9 if channels==1 else 17)
 for high in [False,True]:
  base=(samples//8)*640000//rate if high else len(original[0])
  frames=[]
  for i,source_frame in enumerate(original):
   frame=bytearray(source_frame);frame[2]&=15
   # Changing the bitrate-index changes the protected header CRC.
   frame[4:6]=struct.pack('>H',crc(frame[2:4]+frame[6:6+side]))
   size=base+(i%2);assert len(frame)<=size
   frame.extend(bytes(size-len(frame)))
   if i==0:
    fake=base//2
    sidebits=''.join(f'{v:08b}'for v in frame[6:6+side])
    offset=(18 if channels==1 else 20)if mpeg1 else(9 if channels==1 else 10)
    parts=sum(int(sidebits[offset+i*(59 if mpeg1 else 63):offset+i*(59 if mpeg1 else 63)+12],2)for i in range((2 if mpeg1 else 1)*channels))
    assert fake>=6+side+(parts+7)//8, 'false sync must be outside all granule data' 
    frame[fake:fake+4]=frame[:4] # CRC/side bytes remain zero: fake is not a frame.
   frames.append(bytes(frame))
  case=f'{name}-'+('640k'if high else 'control')
  files[case+'.mp3']=b''.join(frames);files[case+'.mkv']=movie(frames,rate,channels,samples)
  files[case+'.f32le']=(S/(name+'.f32le')).read_bytes()
  cases.append({'name':case,'rate':rate,'channels':channels,'sizes':list(map(len,frames)),'frames':4,'samples':samples*4,'false_sync':base//2})
  if high and channels==1 and rate in [48000,24000,8000]:
   single=case+'-single';files[single+'.mp3']=frames[0];files[single+'.mkv']=movie(frames[:1],rate,channels,samples)
   files[single+'.f32le']=files[case+'.f32le'][:samples*channels*4]
   cases.append({'name':single,'rate':rate,'channels':channels,'sizes':[len(frames[0])],'frames':1,'samples':samples,'false_sync':base//2})
  if not high and rate==48000 and channels==1:
   damaged=list(frames);f=bytearray(damaged[1]);f[4]^=1;damaged[1]=bytes(f)
   files['bad-second-crc.mp3']=b''.join(damaged);files['bad-second-crc.mkv']=movie(damaged,rate,channels,samples)
for name,data in files.items():(D/name).write_bytes(data)
(D/'manifest.json').write_text(json.dumps({'provenance':'FVid-authored compressed chains with free bitrate indices, recomputed CRCs, neutral VP9 video and inherited independent PCM; no encoder/private media/network','cases':cases,'invalid':['bad-second-crc.mp3','bad-second-crc.mkv'],'files':{name:hashlib.sha256(raw).hexdigest()for name,raw in files.items()}},indent=2)+'\n')
print(len(cases),'free-format MP3 cases')
