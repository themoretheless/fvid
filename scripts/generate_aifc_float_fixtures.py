#!/usr/bin/env python3
"""Authored IEEE AIFC samples, with existing authored AVI video companions.
No private input, codec executable or network is used.
"""
import hashlib,json,struct
from pathlib import Path
from generate_g711_fixtures import chunk
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
items=[]
for bits in [32,64]:
 for channels in [1,2]:
  name=f'aifc-float-{bits}-{channels}ch'
  values=[0.,-0.,0.25,-0.5,1.,-1.,2.,-3.,2**-100]
  samples=[values[(frame+channel*3)%len(values)] for frame in range(4103) for channel in range(channels)]
  expected=b''.join(struct.pack('<f',v) for v in samples)
  (D/f'{name}.f32le').write_bytes(expected)
  for upper in [False,True]:
   tag=f'fl{bits}'.encode();tag=tag.upper() if upper else tag
   comm=struct.pack('>HIH',channels,4103,bits)+struct.pack('>HQ',0x400b,8000<<51)+tag+b'\x04IEEE\0'
   data=b''.join(struct.pack('>f' if bits==32 else '>d',v) for v in samples)
   body=b'AIFC'+chunk(b'FVER',struct.pack('>I',0xa2805140),True)+chunk(b'COMM',comm,True)+chunk(b'SSND',bytes(8)+data,True)
   file=f'{name}{"-uppercase" if upper else ""}.aifc'
   output=b'FORM'+struct.pack('>I',len(body))+body
   (D/file).write_bytes(output)
   items.append(dict(file=file,sha256=hashlib.sha256(output).hexdigest(),pcm=f'{name}.f32le',sample_frames=4103,channels=channels,sample_rate=8000,video_companion=f'g711-alaw-{channels}ch.avi'))
(D/'aifc-float.json').write_text(json.dumps(dict(fixtures=items),indent=2)+'\n')
