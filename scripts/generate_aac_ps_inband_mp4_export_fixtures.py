#!/usr/bin/env python3
"""Original implicit PS MP4 export ranges; retain owned packets and offsets."""
import json,struct,hashlib
from generate_he_aac_packet_fixtures import DEST,boxes,box
from generate_aac_ps_inband_fixtures import config

def descriptor(tag,data):
 assert len(data)<128
 return bytes([tag,len(data)])+data

def rewrite(tag,body,setup):
 if tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
  body=b''.join(rewrite(k,p,setup) for k,p in boxes(body))
 elif tag==b'stsd':
  entries=list(boxes(body[8:]))
  if entries[0][0]==b'mp4a':
   entry=bytearray(entries[0][1][:28]);struct.pack_into('>H',entry,16,1)
   esds=bytes(4)+descriptor(3,b'\x00\x02\x00'+descriptor(4,b'\x40\x15'+bytes(11)+descriptor(5,setup))+descriptor(6,b'\x02'))
   body=body[:8]+box(b'mp4a',bytes(entry)+box(b'esds',esds))
 return box(tag,body)

def without_edits(tag,body):
 if tag==b'edts':return b''
 if tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
  body=b''.join(without_edits(k,p) for k,p in boxes(body))
 return box(tag,body)

def main():
 source=json.loads((DEST/'aac-ps-mp4-export-oracles.json').read_text());out=[]
 for c in source['cases']:
  size=960 if '960' in c['file'] else 1024
  raw=(DEST/c['file']).read_bytes();outer=list(boxes(raw))
  assert [k for k,_ in outer].index(b'moov')>[k for k,_ in outer].index(b'mdat')
  for kind in ['LC','SBR']:
   setup=config(size//64,48000,kind)
   data=b''.join(rewrite(k,p,setup) for k,p in outer)
   name=f'he-aac-ps-inband-export-{kind.lower()}-{size}-synthetic.mp4'
   (DEST/name).write_bytes(data)
   unedited=name.replace('inband-export','inband-unedited')
   plain=b''.join(without_edits(k,p) for k,p in boxes(data));(DEST/unedited).write_bytes(plain)
   row=dict(c);row.update(file=name,kind=kind,asc=setup.hex(),sha256=hashlib.sha256(data).hexdigest(),no_edit_file=unedited,no_edit_sha256=hashlib.sha256(plain).hexdigest());out.append(row)
 (DEST/'aac-ps-inband-mp4-export-oracles.json').write_text(json.dumps(dict(cases=out),indent=2)+'\n')
 print('original implicit PS MP4 exports:',len(out))
if __name__=='__main__':main()
