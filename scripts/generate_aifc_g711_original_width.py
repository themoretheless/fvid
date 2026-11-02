"""Authored AIFC original-width acceptance/refusal controls;generation only."""
from pathlib import Path
import struct,json,hashlib
from generate_g711_fixtures import level
from generate_vp9_chroma import mp4,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aifc-g711-original-width'
def chunk(k,b):return k+struct.pack('>I',len(b))+b+bytes(len(b)%2)
def aifc(tag,bits,ch,raw,frames=256):
 comm=struct.pack('>HIH',ch,frames,bits)+struct.pack('>HQ',0x400b,8000<<51)+tag+b'\x04G711\0';body=b'AIFC'+chunk(b'FVER',struct.pack('>I',0xa2805140))+chunk(b'COMM',comm)+chunk(b'SSND',bytes(8)+raw);return b'FORM'+struct.pack('>I',len(body))+body

def generate():
 files={'neutral-video.mp4':mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))};cases=[];invalid=[]
 for law,tag in [('alaw',b'alaw'),('mulaw',b'ulaw')]:
  files[law+'-table.f32le']=b''.join(struct.pack('<f',level(law,i)/32768)for i in range(256))
  for ch in [1,2]:
   raw=bytes((i if c==0 else 255-i)for i in range(256)for c in range(ch));oracle=law+f'-{ch}ch.f32le';files[oracle]=b''.join(struct.pack('<f',level(law,b)/32768)for b in raw)
   for upper in [False,True]:
    for bits in [16,1,8,12,13,14,24,32]:
     name=f'{law}-{ch}ch-{bits}bit'+('-upper'if upper else'')+'.aifc';files[name]=aifc(tag.upper()if upper else tag,bits,ch,raw);cases.append(dict(file=name,law=law,channels=ch,original_bits=bits,rate=8000,frames=256,oracle=oracle))
  for bits in [0,33,65535]:
   name=f'{law}-{bits}bit-invalid.aifc';files[name]=aifc(tag,bits,1,bytes(range(256)));invalid.append(name)
 files['orphan-channel.aifc']=aifc(b'alaw',8,2,bytes(range(256))*2+b'\x55')
 files['truncated-declared-tail.aifc']=aifc(b'ulaw',8,1,bytes(range(256)),257)
 files['manifest.json']=(json.dumps(dict(provenance='Own all-code G711 and independent segment-level PCM oracle. AIFC is audio-only:the short neutral VP9 companion is an independent video control;the AIFC files reproduce the specific audio parser failure. No private media/parameters,FFmpeg or network.',specification='https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-C.9.26.91.pdf',section='4.0 Common Chunk,PDF page14',video='neutral-video.mp4',cases=cases,invalid_original_sizes=invalid,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
