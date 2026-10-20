#!/usr/bin/env python3
"""Authored AU signed8/IEEE samples and saved float PCM; no external codecs."""
import hashlib,json,struct
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
items=[]
for encoding,bits in [(2,8),(6,32),(7,64)]:
 for channels in [1,2]:
  name=f'au-pcm-{bits}-{channels}ch';samples=[];data=bytearray()
  for frame in range(4103):
   for channel in range(channels):
    if bits==8:
     v=(frame+channel*73)%256-128;data+=struct.pack('b',v);samples.append(v/128.)
    else:
     v=[0.,-0.,.25,-.5,1.,-1.,2.,-3.,2**-100][(frame+channel*3)%9]
     data+=struct.pack('>f'if bits==32 else'>d',v);samples.append(v)
  output=b'.snd'+struct.pack('>IIIII',28,len(data),encoding,8000,channels)+bytes(4)+data
  file=f'{name}.au';(D/file).write_bytes(output)
  (D/f'{name}.f32le').write_bytes(b''.join(struct.pack('<f',v)for v in samples))
  items.append(dict(file=file,sha256=hashlib.sha256(output).hexdigest(),sample_frames=4103,channels=channels,sample_rate=8000,video_companion=f'g711-alaw-{channels}ch.avi'))
(D/'au-pcm.json').write_text(json.dumps(dict(fixtures=items),indent=2)+'\n')
