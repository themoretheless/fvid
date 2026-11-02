"""Authored indexed-audio EOF reproducer; generation only."""
from pathlib import Path
import struct,json,hashlib
from generate_aiff_block_padding import audio
from generate_vp9_chroma import mp4,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-index-eof'
def generate():
 files={'neutral-video.mp4':mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))};cases=[]
 for count in [1,2048,2051]:
  for ch in [1,2]:
   values=[((i*157+c*71)%65536)-32768 for i in range(count)for c in range(ch)]
   big=b''.join(struct.pack('>h',v)for v in values);little=b''.join(struct.pack('<h',v)for v in values);oracle=f'{count}-{ch}ch.f32le';files[oracle]=b''.join(struct.pack('<f',v/32768)for v in values)
   def chunk(k,b):return k+struct.pack('<I',len(b))+b+bytes(len(b)%2)
   body=b'WAVE'+chunk(b'fmt ',struct.pack('<HHIIHH',1,ch,8000,8000*ch*2,ch*2,16))+chunk(b'data',little)
   for kind,data in [('wav',b'RIFF'+struct.pack('<I',len(body))+body),('aiff',audio(b'AIFF',16,ch,big,count,0,0)),('au',b'.snd'+struct.pack('>IIIII',24,len(big),3,8000,ch)+big)]:
    name=f'{count}-{ch}ch.{kind}';files[name]=data;cases.append(dict(file=name,kind=kind,frames=count,channels=ch,oracle=oracle))
 files['manifest.json']=(json.dumps(dict(provenance='Own mathematical PCM16,mono/stereo,single/exact/partial packet windows. The audio-only formats use a separate authored short neutral VP9 control. No private media/parameters,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
