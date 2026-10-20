#!/usr/bin/env python3
"""Authored MPEG Layer II frames and independent convolution PCM.
No encoder, private source media or network is needed.
"""
import math,re,struct,json,hashlib
from pathlib import Path
R=Path(__file__).resolve().parents[1];D=R/'tests/fixtures/playback-errors/mp2-native';D.mkdir(exist_ok=True)
class Writer:
 def __init__(self):self.bits=[]
 def put(self,v,n):self.bits.extend((v>>i)&1 for i in range(n-1,-1,-1))
 def packed(self):
  bits=self.bits+[0]*(-len(self.bits)%8)
  return bytes(sum(bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(bits),8))
# Normative quantizer levels by allocation family; zero means unallocated.
rows={
 'a':[0,3]+[(1<<n)-1 for n in range(3,17)],
 'b':[0,3,5,7,9]+[(1<<n)-1 for n in range(4,14)]+[65535],
 'c':[0,3,5,7,9,15,31,65535], 'd':[0,3,5,65535],
 'e':[0,3,5,9]+[(1<<n)-1 for n in range(4,16)],
 'f':[0,3,5,9,15,31,63,127],
 'g':[0,3,5,7,9]+[(1<<n)-1 for n in range(4,15)],'h':[0,3,5,9]}
window=[int(v)for v in re.findall(r'-?\d+',(R/'crates/fvid-codecs/src/mpeg_synthesis_window.rs').read_text().split('= [',1)[1])];assert len(window)==512
kernel=[[[window[(lag//2)*64+(lag%2)*32+j]/65536.*math.cos(math.pi*(16+j+(32 if lag%2 else 0))*(2*k+1)/64.)for k in range(32)]for j in range(32)]for lag in range(16)]
def convolve(slots,channels):
 nonzero=[[[ (k,v)for k,v in enumerate(row[ch])if v]for ch in range(channels)]for row in slots]
 return [sum(kernel[lag][j][k]*v for lag in range(min(16,t+1)) for k,v in nonzero[t-lag][ch])for t in range(len(slots))for j in range(32)for ch in range(channels)]
def crc(bits):
 c=65535
 for bit in bits:
  feedback=(c>>15)^bit;c=(c<<1)&65535
  if feedback:c^=0x8005
 return c
records=[]
free_records=[]
settings=[(3,1,64,27,[(0,'a'),(3,'b'),(11,'c'),(23,'d')]),(3,0,96,30,[(0,'a'),(3,'b'),(11,'c'),(23,'d')]),(3,0,32,8,[(0,'e'),(2,'f')]),(3,2,32,12,[(0,'e'),(2,'f')]),(2,0,32,30,[(0,'g'),(4,'f'),(11,'h')]),(0,0,32,30,[(0,'g'),(4,'f'),(11,'h')])]
def row_for(setting,sb):
 ver,rate,per,limit,groups=setting
 return rows[next(family for start,family in reversed(groups)if sb>=start)]
for setting_id,setting in enumerate(settings):
 ver,rate_index,per,limit,groups=setting
 for sb,family in groups:
  for code,levels in enumerate(rows[family][1:],1):
   mode=code%4;channels=1 if mode==3 else 2;bound=min(4*(1+(code%4)),limit)if mode==1 else limit
   protected=code%2==1;sample_rate=[44100,48000,32000][rate_index]>>(0 if ver==3 else 1 if ver==2 else 2)
   rates=[0,32,48,56,64,80,96,112,128,160,192,224,256,320,384,0]if ver==3 else[0,8,16,24,32,40,48,56,64,80,96,112,128,144,160,0]
   bitrate=per*channels;index=rates.index(bitrate)
   h=(2047<<21)|(ver<<19)|(2<<17)|((not protected)<<16)|(index<<12)|(rate_index<<10)|((code%2)<<9)|(mode<<6)|((code%4)<<4)
   header=h.to_bytes(4,'big');frame_bytes=144000*bitrate//sample_rate+code%2;frames=[];slots=[];protected_runs=[]
   for frame in range(2):
    w=Writer()
    for band in range(limit):
     width=(len(row_for(setting,band))-1).bit_length()
     for ch in range(channels if band<bound else 1):w.put(code if band==sb else 0,width)
    selections=[(code+ch)%4 for ch in range(channels)]
    for ch in range(channels):w.put(selections[ch],2)
    protected_bits=w.bits[:];protected_runs.append(protected_bits)
    scalefactors=[]
    for ch,selection in enumerate(selections):
     a,b,c=6+ch*3,9+ch*3,12+ch*3
     encoded={0:[a,b,c],1:[a,c],2:[a],3:[a,b]}[selection]
     for value in encoded:w.put(value,6)
     scalefactors.append({0:[a,b,c],1:[a,a,c],2:[a,a,a],3:[a,b,b]}[selection])
    for group in range(12):
     band_values=[]
     for ch in range(channels if sb<bound else 1):
      values=[((group*3+i+frame*5+ch)%3)*((levels-1)//2)for i in range(3)]
      if levels in [3,5,9]:w.put(sum(v*(levels**i)for i,v in enumerate(values)),{3:5,5:7,9:10}[levels])
      else:
       for v in values:w.put(v,(levels+1).bit_length()-1)
      band_values.append(values)
     for i in range(3):
      row=[[0.]*32 for _ in range(channels)]
      for ch in range(channels):
       v=band_values[ch if sb<bound else 0][i]
       row[ch][sb]=(2*v+1-levels)/levels*2**(1-scalefactors[ch][group//4]/3)
      slots.append(row)
    prefix=header
    if protected:
     header_bits=[(v>>i)&1 for v in header[2:]for i in range(7,-1,-1)]
     prefix+=struct.pack('>H',crc(header_bits+protected_bits))
    payload=prefix+w.packed();assert len(payload)<=frame_bytes,(setting_id,sb,code,len(payload),frame_bytes)
    frames.append(payload+bytes(frame_bytes-len(payload)))
   name=f'{setting_id}-{sb}-{code}';raw=b''.join(frames);pcm=b''.join(struct.pack('<f',v)for v in convolve(slots,channels))
   (D/f'{name}.mp2').write_bytes(raw);(D/f'{name}.f32le').write_bytes(pcm)
   (D/f'{name}.meta').write_bytes(struct.pack('>IBI',sample_rate,channels,frame_bytes))
   records.append(dict(name=name,rate=sample_rate,channels=channels,frame_bytes=frame_bytes,sha256=hashlib.sha256(raw).hexdigest(),pcm_sha256=hashlib.sha256(pcm).hexdigest()))
   if setting_id in [0,1,4,5] and code in [1,3,7,15]:
    for free_rate in ([sample_rate,32000] if setting_id==1 else [sample_rate]):
     free_name=f'free-{setting_id}-{sb}-{code}-{free_rate}'
     base=frame_bytes-(code%2)+7;free_frames=[]
     for j,pad in enumerate([0,1,0,1]):
      source=frames[j%2];word=int.from_bytes(source[:4],'big')&~((15<<12)|(1<<9))
      word|=pad<<9
      if free_rate==32000 and sample_rate!=32000:word=(word&~(3<<10))|(2<<10)
      prefix=word.to_bytes(4,'big')
      if protected:
       bits=[(v>>i)&1 for v in prefix[2:]for i in range(7,-1,-1)]
       prefix+=struct.pack('>H',crc(bits+protected_runs[j%2]))
      packet=bytearray(prefix+source[6 if protected else 4:]);packet.extend(bytes(base+pad-len(packet)))
      # One false sync in ancillary bytes must not win over a confirmed chain.
      if j==0:packet[-12:-8]=prefix[:4]
      free_frames.append(bytes(packet))
     encoded=b''.join(free_frames);expected=b''.join(struct.pack('<f',v)for v in convolve(slots*2,channels))
     (D/f'{free_name}.mp2').write_bytes(encoded);(D/f'{free_name}.f32le').write_bytes(expected)
     (D/f'{free_name}.ffmeta').write_bytes(struct.pack('>IB4I',free_rate,channels,*map(len,free_frames)))
     free_records.append(dict(name=free_name,rate=free_rate,channels=channels,sizes=list(map(len,free_frames)),sha256=hashlib.sha256(encoded).hexdigest(),pcm_sha256=hashlib.sha256(expected).hexdigest()))

(D/'free-manifest.json').write_text(json.dumps(dict(cases=free_records),indent=2)+'\n')
(D/'manifest.json').write_text(json.dumps(dict(cases=records,video_companion='../g711-alaw-2ch.avi'),indent=2)+'\n')
print(len(records),'authored MP2 cases and',len(free_records),'free-format cases')
# Full video reproducers for free-format audio, authored AVC PCM fields.
from generate_pcm_precision_fixtures import element
from generate_avc_field_pcm_samples import configuration,field
config=configuration(8)
def integer(identifier,value):return element(identifier,value.to_bytes(max(1,(value.bit_length()+7)//8),'big'))
video=integer('d7',1)+integer('83',1)+element('86',b'V_MPEG4/ISO/AVC')+element('63a2',config)+element('e0',integer('b0',32)+integer('ba',32))
for record in free_records:
 name=record['name'];raw=(D/f'{name}.mp2').read_bytes();rate=record['rate'];channels=record['channels']
 audio=integer('d7',2)+integer('83',2)+element('86',b'A_MPEG/L2')+integer('23e383',1152*1000000000//rate)+element('e1',element('b5',struct.pack('>d',rate))+integer('9f',channels))
 clusters=[]
 for i,bottom in enumerate([False,True]):
  nal=field(bottom,8,i==0);payload=len(nal).to_bytes(4,'big')+nal
  clusters.append((i*40000000,1,payload))
 at=0
 for i,size in enumerate(record['sizes']):clusters.append((i*1152*1000000000//rate,2,raw[at:at+size]));at+=size
 body=element('1549a966',integer('2ad7b1',1))+element('1654ae6b',element('ae',video)+element('ae',audio))
 for timestamp,track,payload in sorted(clusters):body+=element('1f43b675',integer('e7',timestamp)+element('a3',bytes([128+track,0,0,128])+payload))
 output=element('1a45dfa3',element('4282',b'matroska'))+element('18538067',body)
 (D/f'{name}.mkv').write_bytes(output);record['video_sha256']=hashlib.sha256(output).hexdigest()
(D/'free-manifest.json').write_text(json.dumps(dict(cases=free_records),indent=2)+'\n')
# Damaged second frame must not make synchronization combine pairs of frames.
record=next(v for v in free_records if v['name']=='free-0-0-1-48000')
bad=bytearray((D/(record['name']+'.mp2')).read_bytes());bad[record['sizes'][0]+4]^=1
(D/'free-second-crc-damaged.mp2').write_bytes(bad)

valid=(D/(record['name']+'.mp2')).read_bytes()
second=valid[record['sizes'][0]:record['sizes'][0]+record['sizes'][1]]
video=bytearray((D/(record['name']+'.mkv')).read_bytes());offset=video.index(second);video[offset+4]^=1
(D/'free-second-crc-damaged.mkv').write_bytes(video)
manifest=json.loads((D/'free-manifest.json').read_text())
manifest['invalid_cases']=[dict(file='free-second-crc-damaged.'+ext,sha256=hashlib.sha256((D/('free-second-crc-damaged.'+ext)).read_bytes()).hexdigest())for ext in ['mp2','mkv']]
(D/'free-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
