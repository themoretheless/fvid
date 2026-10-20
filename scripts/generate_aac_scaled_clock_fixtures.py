#!/usr/bin/env python3
"""Owned AAC/VP9 TrackTimestampScale regressions, explicit offline generation."""
import hashlib,json,struct
from fractions import Fraction
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'aac-scaled-clock'
def uint(tag,n):return ebml(tag,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def packets(raw):
 parts=[];at=0
 while at<len(raw):
  h=raw[at:at+7];n=((h[3]&3)<<11)|(h[4]<<3)|(h[5]>>5);assert n>=7;parts.append(raw[at+7:at+n]);at+=n
 assert at==len(raw);return parts
def video(asc,parts,factor,shift,lace,gap,frame_samples,channels,preroll=0):
 video=ebml('ae',uint('d7',1)+uint('73c5',1)+uint('83',1)+ebml('86',b'V_VP9')+ebml('e0',uint('b0',16)+uint('ba',16)))
 audio=ebml('ae',uint('d7',2)+uint('73c5',2)+uint('83',2)+ebml('86',b'A_AAC')+ebml('63a2',asc)+ebml('23314f',struct.pack('>d',factor))+(uint('23e383',(frame_samples*1000000000+47999)//48000)if lace else b'')+ebml('e1',ebml('b5',struct.pack('>d',48000))+uint('9f',channels)))
 clusters=b'';groups=[]
 for start in range(0,len(parts),3 if lace else 1):
  count=3 if lace else 1;cluster=([0,1,4,3,2,5][start%6]if shift else 0)
  exact=(Fraction((start*frame_samples-preroll+(gap if start>=3 else 0))*1000000000,48000)-cluster*1000000)/(1000000*Fraction.from_float(factor));relative=round(exact)if start==0 and preroll else exact.numerator//exact.denominator
  payload=b'\x82'+struct.pack('>hB',relative,2 if lace else 0x80)
  if lace:
   payload+=bytes([count-1])
   for p in parts[start:start+count-1]:payload+=b'\xff'*(len(p)//255)+bytes([len(p)%255])
  payload+=b''.join(parts[start:start+count])
  clusters+=ebml('1f43b675',uint('e7',cluster)+ebml('a3',b'\x81'+struct.pack('>hB',start*((frame_samples*1000+47999)//48000)-cluster,0x80)+frame(8,'420',16,16))+ebml('a3',payload))
  groups.append(dict(first=start,count=count,cluster=cluster,relative=relative))
 data=ebml('1a45dfa3',ebml('4282',b'matroska')+uint('4287',3)+uint('4285',2))+ebml('18538067',ebml('1549a966',uint('2ad7b1',1000000))+ebml('1654ae6b',video+audio)+clusters)
 return data,groups
def generate():
 raw=(ROOT/'aac-height-normal-ten.aac').read_bytes();parts=packets(raw)
 assert len(parts)==6
 asc=(ROOT/'aac-device-layout/normal-ten.asc').read_bytes();cases=[];files={}
 variants=[('double',2.,False,False),('half',.5,False,False),('third',1/3,False,False),('near-unit',1.00000000001,False,False),('cluster-shift',2.,True,False),('lace-double',2.,False,True),('lace-third',1/3,True,True),('double-gap',2.,False,False),('lace-gap',2.,False,True)]
 for name,factor,shift,lace in variants:
  gap=96 if name.endswith('-gap')else 0
  data,groups=video(asc,parts,factor,shift,lace,gap,1024,10)
  filename=name+'.mkv';files[filename]=data;cases.append(dict(file=filename,factor=factor,groups=groups,video_frames=len(groups),samples=6144+gap,coded_samples=6144,gap_samples=gap,channels=10,frame_samples=1024,profile='lc',pts_ns=[(i*1024+(gap if i>=3 else 0))*1000000000//48000 for i in range(6)],factor_be=struct.pack('>d',factor).hex()))
 # Existing authored Main-PS transports qualify delayed scaled/laced metadata.
 for c in json.loads((ROOT/'aac-main-ps-adts.json').read_text())['cases']:
  control=(ROOT/c['adts']).read_bytes();ps_parts=packets(control)
  data,groups=video(bytes.fromhex(c['asc']),ps_parts,1/3,True,True,0,2048,2)
  name='ps-'+c['name']+'.mkv';files[name]=data
  cases.append(dict(file=name,factor=1/3,groups=groups,video_frames=len(groups),samples=24576,coded_samples=24576,gap_samples=0,channels=2,frame_samples=2048,profile='ps',control=c['adts'],control_sha256=hashlib.sha256(control).hexdigest(),pts_ns=[i*2048*1000000000//48000 for i in range(12)],factor_be=struct.pack('>d',1/3).hex()))
 # Compose negative source preroll with shifted fractional PS lacing.
 for c in json.loads((ROOT/'aac-main-ps-adts.json').read_text())['cases']:
  control=(ROOT/c['adts']).read_bytes();data,groups=video(bytes.fromhex(c['asc']),packets(control),1/3,True,True,0,2048,2,preroll=480)
  name='ps-'+c['name']+'-preroll.mkv';files[name]=data
  cases.append(dict(file=name,factor=1/3,groups=groups,video_frames=len(groups),samples=24096,coded_samples=24576,gap_samples=0,preroll_samples=480,channels=2,frame_samples=2048,profile='ps',control=c['adts'],control_sha256=hashlib.sha256(control).hexdigest(),pts_ns=[(i*2048-480)*1000000000//48000 for i in range(12)],factor_be=struct.pack('>d',1/3).hex()))
 control=raw*12;files['warm-third.aac']=control;warm=packets(control)
 data,groups=video(asc,warm,1/3,False,False,0,1024,10);files['warm-third.mkv']=data
 cases.append(dict(file='warm-third.mkv',factor=1/3,groups=groups,video_frames=len(groups),samples=73728,coded_samples=73728,gap_samples=0,channels=10,frame_samples=1024,profile='lc',control='aac-scaled-clock/warm-third.aac',control_sha256=hashlib.sha256(control).hexdigest(),pts_ns=[i*1024*1000000000//48000 for i in range(72)],factor_be=struct.pack('>d',1/3).hex()))
 files['manifest.json']=(json.dumps(dict(cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
