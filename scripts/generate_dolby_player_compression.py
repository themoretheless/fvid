"""Owned player-worker RF witnesses reusing short repository synthetic media."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'dolby-player-compression'
def generate():
 files={};cases=[]
 for label,folder,name,extensions,mask in [
  ('ac3-dual','ac3-rf-compression','l0-1-r1-c127',['mkv','mp4'],0xb),
  ('ac3-coupled','ac3-rf-compression','coupled-c127',['mkv','mp4'],3),
  ('eac3-dual','eac3-rf-compression','l0-1-c127-transient',['mkv'],0xb),
  ('eac3-expanded','eac3-rf-expanded','51-lc-44100-6-1-0-switch',['mkv','mp4'],0x63f)]:
  for ext in extensions:
   filename=f'{label}.{ext}';files[filename]=(ROOT/folder/f'{name}.{ext}').read_bytes();cases.append(dict(file=filename,mask=mask,seek_ms=40))
 # Non-Dolby PCM must ignore a session's Dolby policy.
 import struct
 from generate_eac3_presentation import element,uint,frame
 v=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 a=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_PCM/INT/LIT')+element(0xe1,element(0xb5,struct.pack('>d',48000.))+uint(0x9f,2)+uint(0x6264,16))
 signal=b''.join(struct.pack('<hh',((n%31)-15)*128,-((n%31)-15)*128)for n in range(1536))
 clusters=b''.join(element(0x1f43b675,uint(0xe7,i*32)+element(0xa3,b'\x81\0\0\x80'+frame(8,'420',16,16))+element(0xa3,b'\x82\0\0\x80'+signal))for i in range(3))
 files['pcm.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',96.)))+element(0x1654ae6b,element(0xae,v)+element(0xae,a))+clusters)
 cases.append(dict(file='pcm.mkv',mask=None,seek_ms=40,dolby=False))
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
