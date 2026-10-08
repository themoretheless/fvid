#!/usr/bin/env python3
"""Original PS MP4s with LC-core sample-entry clock and unchanged output ASC."""
import struct,json,hashlib
from generate_he_aac_packet_fixtures import DEST,boxes,box

def rewrite_audio(tag,body):
 if tag in [b'trak',b'mdia',b'minf',b'stbl',b'edts']:
  body=b''.join(rewrite_audio(k,p) for k,p in boxes(body))
 elif tag==b'mdhd':
  body=bytearray(body);assert body[0]==0
  rate,duration=struct.unpack_from('>II',body,12);assert rate==48000 and duration%2==0
  struct.pack_into('>II',body,12,24000,duration//2);body=bytes(body)
 elif tag==b'stsd':
  entry=list(boxes(body[8:]))[0];assert entry[0]==b'mp4a'
  audio=bytearray(entry[1]);struct.pack_into('>H',audio,16,1);struct.pack_into('>I',audio,24,24000<<16)
  body=body[:8]+box(b'mp4a',bytes(audio))
 elif tag==b'stts':
  body=bytearray(body)
  for off in range(8,len(body),8):
   duration=struct.unpack_from('>I',body,off+4)[0];assert duration%2==0;struct.pack_into('>I',body,off+4,duration//2)
  body=bytes(body)
 elif tag==b'elst':
  body=bytearray(body);assert body[0]==0
  for off in range(8,len(body),12):
   media=struct.unpack_from('>i',body,off+4)[0]
   if media>=0:assert media%2==0;struct.pack_into('>i',body,off+4,media//2)
  body=bytes(body)
 return box(tag,body)

def main():
 explicit=json.loads((DEST/'aac-ps-mp4-export-oracles.json').read_text())['cases']
 implicit=json.loads((DEST/'aac-ps-inband-mp4-export-oracles.json').read_text())['cases']
 out=[]
 for c in explicit+[c for c in implicit if c['kind']=='SBR']:
  raw=(DEST/c['file']).read_bytes();outer=list(boxes(raw));assert [k for k,_ in outer].index(b'moov')>[k for k,_ in outer].index(b'mdat')
  rebuilt=[]
  for tag,body in outer:
   if tag==b'moov':
    children=[]
    for k,p in boxes(body):
     audio=k==b'trak' and any(a==b'mdia' and any(b==b'hdlr' and q[8:12]==b'soun' for b,q in boxes(d)) for a,d in boxes(p))
     children.append(rewrite_audio(k,p) if audio else box(k,p))
    body=b''.join(children)
   rebuilt.append(box(tag,body))
  data=b''.join(rebuilt);name=c['file'].replace('export','core-clock',1);(DEST/name).write_bytes(data)
  row=dict(c);row.update(file=name,source=c['file'],container_rate=24000,output_rate=48000,sha256=hashlib.sha256(data).hexdigest());out.append(row)
 (DEST/'aac-ps-core-clock-oracles.json').write_text(json.dumps(dict(cases=out),indent=2)+'\n')
 print('original core-clock PS videos:',len(out))
if __name__=='__main__':main()
