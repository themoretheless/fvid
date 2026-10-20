"""Authored ALAC ramps and neutral VP9 for absolute timeline/seek tests."""
import struct,json,hashlib
from pathlib import Path
from generate_audio_resample_window_fixture import ebml,packet,word
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-timeline'
def u(t,n):return ebml(t,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def generate():
 files={};cookie=bytearray(24);cookie[:4]=word(128);cookie[5:10]=bytes([16,40,10,14,1]);cookie[20:24]=word(48000)
 audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
 video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
 for name,gap,delay,tail in [('offset',0,0,0),('offset-gap',48,0,0),('offset-delay-tail',0,17,13),('offset-discard-last',0,0,128)]:
  size=96 if gap else 128
  entry=audio if not delay else ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+u('56aa',354167)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
  blocks=u('e7',30000)
  for i in range(4):
   tick=(2400+i*size+(gap if i>=2 else 0))*1000000000//48000//1000
   values=[(n*119)%24001-12000 for n in range(i*size,(i+1)*size)]
   payload=b'\x82'+struct.pack('>hB',tick-30000,128)+packet(values)
   audio_block=ebml('a0',ebml('a1',payload)+ebml('75a2',round(tail*1000000000/48000).to_bytes(8,'big',signed=True)))if i==3 and tail else ebml('a3',payload)
   blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*3000-30000,128)+frame(8,'420',16,16))+audio_block
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',4 if delay or tail else 3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+entry)+ebml('1f43b675',blocks))
 files['manifest.json']=(json.dumps(dict(provenance='Own uncompressed ALAC packet writer, independent ramp formula and neutral VP9. Four mono packets (128samples for offset,96 for gap) at48kHz,50ms origin,optional48sample gap. No private data/FFmpeg/network.',files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
