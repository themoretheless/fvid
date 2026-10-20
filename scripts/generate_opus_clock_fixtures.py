"""Own Opus DTX/VP9 one-tick gap/origin regressions, offline."""
import struct,json,hashlib
from pathlib import Path
from fractions import Fraction
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/opus-clock'
def u(t,n):return ebml(t,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def generate():
 files={}
 for name,stamps in [('gap',[0,21,41]),('offset',[1,21,41])]:
  head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
  audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_OPUS')+ebml('63a2',head)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
  video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
  blocks=u('e7',0)
  for stamp in stamps:blocks+=ebml('a3',b'\x81'+struct.pack('>hB',stamp,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',stamp,128)+bytes.fromhex('f8fffe'))
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
 head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
 audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_OPUS')+ebml('63a2',head)+ebml('23314f',struct.pack('>d',1/3))+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
 blocks=u('e7',0)
 for i,tick in enumerate([0,7,15,22]):blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*3,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',tick,128)+bytes.fromhex('80fffe'))
 files['short-third.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
 for name,lace in [('short-shift-third',False),('short-lace-third',True)]:
  clusters=b''
  for i in range(0,4,2 if lace else 1):
   cluster=i%3
   q=(Fraction(i*5,2)-cluster)/Fraction.from_float(1/3);tick=q.numerator//q.denominator
   payload=bytes.fromhex('80fffe')
   block=b'\x82'+struct.pack('>hB',tick,2 if lace else 128)+(bytes([1,3])+payload*2 if lace else payload)
   clusters+=ebml('1f43b675',u('e7',cluster)+ebml('a3',b'\x81'+struct.pack('>hB',i*3-cluster,128)+frame(8,'420',16,16))+ebml('a3',block))
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000000))+ebml('1654ae6b',video+audio)+clusters)
 for name,head_trim in [('short-delay-tail',0),('short-delay-head-tail',9)]:
  head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',17,48000,0,0)
  audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_OPUS')+ebml('63a2',head)+u('56aa',354167)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
  blocks=u('e7',0)
  for i,tick in enumerate([0,3,5,8]):
   payload=b'\x82'+struct.pack('>hB',tick,0)+bytes.fromhex('80fffe')
   padding=-head_trim if i==2 else 13 if i==3 else 0
   ns=round(abs(padding)*1000000000/48000)*(-1 if padding<0 else 1)
   ab=ebml('a0',ebml('a1',payload)+ebml('75a2',ns.to_bytes(8,'big',signed=True)))if padding else ebml('a3',payload)
   blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*3,128)+frame(8,'420',16,16))+ab
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',4)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
 files['manifest.json']=(json.dumps(dict(provenance='Authored Opus DTX packets and neutral VP9; no private data or external encoder/network.',files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
