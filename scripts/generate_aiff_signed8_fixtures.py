#!/usr/bin/env python3
"""Synthetic signed8 AIFF/AIFC; no private input or external codecs."""
import hashlib,json,struct
from pathlib import Path
from generate_g711_fixtures import chunk
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
items=[]
for channels in [1,2]:
 values=[(frame+channel*73)%256-128 for frame in range(4103) for channel in range(channels)]
 data=b''.join(struct.pack('b',v)for v in values)
 pcm=b''.join(struct.pack('<f',v/128.)for v in values)
 for coding in ['AIFF','NONE','sowt']:
  name=f'aiff-signed8-{coding}-{channels}ch'
  comm=struct.pack('>HIH',channels,4103,8)+struct.pack('>HQ',0x400b,8000<<51)
  if coding!='AIFF':comm+=coding.encode()+b'\x04PCM8\0'
  body=(b'AIFF'if coding=='AIFF'else b'AIFC')
  if coding!='AIFF':body+=chunk(b'FVER',struct.pack('>I',0xa2805140),True)
  body+=chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+data,True)
  output=b'FORM'+struct.pack('>I',len(body))+body
  file=f'{name}.aiff';(D/file).write_bytes(output);(D/f'{name}.f32le').write_bytes(pcm)
  items.append(dict(file=file,sha256=hashlib.sha256(output).hexdigest(),pcm=f'{name}.f32le',channels=channels,frames=4103,video_companion=f'g711-alaw-{channels}ch.avi'))
(D/'aiff-signed8.json').write_text(json.dumps(dict(fixtures=items),indent=2)+'\n')
