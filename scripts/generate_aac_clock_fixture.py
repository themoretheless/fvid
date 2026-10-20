#!/usr/bin/env python3
"""Offline authored AAC/VP9 video with quantized and discontinuous clocks."""
from pathlib import Path
import struct, json, hashlib
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def generate():
 raw=(ROOT/'aac-height-normal-ten.aac').read_bytes(); packets=[];at=0
 while at<len(raw):
  h=raw[at:at+7];n=((h[3]&3)<<11)|(h[4]<<3)|(h[5]>>5);packets.append(raw[at+7:at+n]);at+=n
 files={}
 asc=(ROOT/'aac-device-layout/normal-ten.asc').read_bytes()
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_AAC')+ebml('63a2',asc)+ebml('e1',ebml('b5',struct.pack('>d',48000))+ebml('9f',b'\x0a')))
 for name,ticks in [('coarse',[0,21,42,64,85,106]),('jumps',[0,21,142,164,10,31]),('gap',[0,21,142,164,185,206]),('tick-gap',[0,21,42,65,86,107]),('long',[i*1024*1000//48000 for i in range(72)])]:
  clusters=b''.join(ebml('1f43b675',ebml('e7',t.to_bytes(2,'big'))+ebml('a3',b'\x81'+struct.pack('>h',i*22-t)+b'\x80'+frame(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+p))for i,(t,p) in enumerate(zip(ticks,(packets*(len(ticks)//len(packets)+1))[:len(ticks)])))
  data=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+clusters)
  filename=f'aac-clock-{name}.mkv'
  (ROOT/filename).write_bytes(data);files[filename]=hashlib.sha256(data).hexdigest()
 for name,discard in [('tail',13),('invalid-tail',1025)]:
  clusters=b''
  for i,p in enumerate(packets):
   t=i*1024*1000//48000;audio_block=ebml('a3',b'\x82\0\0\x80'+p)
   if i==len(packets)-1:
    padding=(discard*1000000000+24000)//48000
    audio_block=ebml('a0',ebml('a1',b'\x82\0\0\0'+p)+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
   clusters+=ebml('1f43b675',ebml('e7',t.to_bytes(2,'big'))+ebml('a3',b'\x81'+struct.pack('>h',i*22-t)+b'\x80'+frame(8,'420',16,16))+audio_block)
  data=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+clusters)
  filename=f'aac-clock-{name}.mkv';(ROOT/filename).write_bytes(data);files[filename]=hashlib.sha256(data).hexdigest()
 for case in json.loads((ROOT/'aac-main-ps-adts.json').read_text())['cases']:
  raw=(ROOT/case['adts']).read_bytes();parts=[];at=0
  while at<len(raw):
   h=raw[at:at+7];n=((h[3]&3)<<11)|(h[4]<<3)|(h[5]>>5);parts.append(raw[at+7:at+n]);at+=n
  audio_ps=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_AAC')+ebml('63a2',bytes.fromhex(case['asc']))+ebml('e1',ebml('b5',struct.pack('>d',48000))+ebml('9f',b'\2')))
  clusters=b''
  for i,p in enumerate(parts):
   t=i*2048*1000//48000;block=ebml('a3',b'\x82\0\0\x80'+p)
   discard=7 if i==len(parts)-2 else 13 if i==len(parts)-1 else 0
   if discard:
    padding=(discard*1000000000+24000)//48000
    block=ebml('a0',ebml('a1',b'\x82\0\0\0'+p)+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
   clusters+=ebml('1f43b675',ebml('e7',t.to_bytes(2,'big'))+ebml('a3',b'\x81'+struct.pack('>h',i*43-t)+b'\x80'+frame(8,'420',16,16))+block)
  data=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio_ps)+clusters)
  filename=f'aac-clock-ps-{case["name"]}-tail.mkv';(ROOT/filename).write_bytes(data);files[filename]=hashlib.sha256(data).hexdigest()
 # Signed trims and declared offsets: literal source-sample clock oracles.
 for name,head_packet,head,tail,offset,gap in [
  ('head',0,17,0,0,0),('middle-head',2,17,13,0,0),
  ('head-gap',2,17,13,0,4768),('offset-head',0,17,13,2400,0),
  ('full-head',0,1024,13,0,0)]:
  clusters=b''
  for i,p in enumerate(packets):
   sample=i*1024+offset+(gap if i>=2 else 0);t=sample*1000//48000
   discard=-head if i==head_packet else tail if i==len(packets)-1 else 0
   block=ebml('a3',b'\x82\0\0\x80'+p)
   if discard:
    padding=(abs(discard)*1000000000+24000)//48000
    if discard<0:padding=-padding
    block=ebml('a0',ebml('a1',b'\x82\0\0\0'+p)+ebml('75a2',padding.to_bytes(8,'big',signed=True)))
   clusters+=ebml('1f43b675',ebml('e7',t.to_bytes(2,'big'))+ebml('a3',b'\x81'+struct.pack('>h',i*22-t)+b'\x80'+frame(8,'420',16,16))+block)
  data=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+clusters)
  filename=f'aac-clock-{name}.mkv';(ROOT/filename).write_bytes(data);files[filename]=hashlib.sha256(data).hexdigest()
 cases={name:dict(sample_rate=48000,frame_samples=2048 if '-ps-'in name else 1024,output_channels=2 if '-ps-'in name else 10,packets=12 if '-ps-'in name else 72 if '-long.'in name else 6)for name in files}
 (ROOT/'aac-clock-generated.json').write_text(json.dumps(dict(cases=cases,files=files),indent=2)+'\n')
if __name__=='__main__':generate()
