"""Owned Matroska E-AC-3 programme selection and packetization witnesses."""
import json,struct
from pathlib import Path
from generate_eac3_presentation import element,uint,frame
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'eac3-matroska-programmes'
def split_movie(raw,rate,blocks,channels):
 v=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 a=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_EAC3')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels))
 clusters=[];cycle=len(raw)//3
 for f in range(3):
  packets=[];at=f*cycle
  while at<(f+1)*cycle:
   size=((raw[at+2]&7)*256+raw[at+3]+1)*2;packets.append(raw[at:at+size]);at+=size
  clusters.append(element(0x1f43b675,uint(0xe7,f*blocks*256*1000//rate)+element(0xa3,b'\x81\0\0\x80'+frame(8,'420',16,16))+b''.join(element(0xa3,b'\x82\0\0\x80'+p)for p in packets)))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',3*blocks*256*1000/rate)))+element(0x1654ae6b,element(0xae,v)+element(0xae,a))+b''.join(clusters))
def generate():
 source=ROOT/'eac3-programme-reader';m=json.loads((source/'manifest.json').read_bytes());files={};cases=[]
 for c in m['cases']:
  files[c['name']+'.mkv']=(source/(c['name']+'.mkv')).read_bytes()
  for control in c['controls']:files[control['name']+'.mkv']=(source/(control['name']+'.mkv')).read_bytes()
  cases.append(c)
  if c['name']in['native-l0-1-r1-b3','expanded-r1-b6-t1']:
   name=c['name']+'-split';first=c['controls'][0];files[name+'.mkv']=split_movie((source/(c['name']+'.ec3')).read_bytes(),first['rate'],first['blocks'],first['channels']);cases.append(dict(name=name,controls=c['controls']))
 c=m['cases'][0];data=(source/(c['name']+'.mkv')).read_bytes();old=element(0xb5,struct.pack('>d',c['controls'][0]['rate']));assert data.count(old)==1
 files['placeholder-rate.mkv']=data.replace(old,element(0xb5,struct.pack('>d',48000)));cases.append(dict(name='placeholder-rate',controls=c['controls']))
 for name in ['corrupt-unselected-p1','corrupt-unselected-p7','without-programme7']:files[name+'.mkv']=(source/(name+'.mkv')).read_bytes()
 files['pcm.mkv']=(ROOT/'dolby-player-compression/pcm.mkv').read_bytes()
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
