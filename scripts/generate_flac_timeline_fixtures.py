"""Own multi-packet FLAC/VP9 fractional timestamp regressions, offline only."""
from pathlib import Path
from fractions import Fraction
import struct,json,hashlib
from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame,stream
from generate_vp9_chroma import frame as video
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-timeline'
def u(tag,n):return ebml(tag,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def generate():
 files={}
 for label,factor in [('unit',1.),('third',1/3),('tail',1.),('discard-last',1.)]:
  packets=[frame(i,64,False,level)for i,level in enumerate([4096,-4096,8192,-8192])]
  private=stream(packets,[64]*4)[:42]
  audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_FLAC')+ebml('63a2',private)+(ebml('23314f',struct.pack('>d',factor))if factor!=1 else b'')+ebml('e1',ebml('b5',struct.pack('>d',44100))+u('9f',1)))
  vt=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
  blocks=u('e7',0)
  for i,p in enumerate(packets):
   q=Fraction(i*64*1000,44100)/Fraction.from_float(factor);tick=q.numerator//q.denominator
   block=b'\x82'+struct.pack('>hB',tick,128)+p
   tail=13 if label=='tail' else 64 if label=='discard-last' else 0
   audio_block=ebml('a0',ebml('a1',block)+ebml('75a2',((tail*1000000000+22050)//44100).to_bytes(8,'big',signed=True)))if i==3 and tail else ebml('a3',block)
   blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*2,128)+video(8,'420',16,16))+audio_block
  files[label+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',4 if label in ['tail','discard-last'] else 3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000000))+ebml('1654ae6b',vt+audio)+ebml('1f43b675',blocks))
 files['manifest.json']=(json.dumps(dict(provenance='Own numbered FLAC constant frames and neutral VP9. Four 64-sample packets at 44100 Hz; literal levels 4096,-4096,8192,-8192. No private data, FFmpeg or network.',files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
