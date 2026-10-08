#!/usr/bin/env python3
"""Own PS Matroska export gaps, reusing authored AVC/SCE/SBR/PS only."""
import json,struct,hashlib
from generate_he_aac_packet_fixtures import DEST

def main():
 source=json.loads((DEST/'aac-ps-matroska-oracles.json').read_text());packets=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();absence=json.loads((DEST/'aac-ps-absence-oracles.json').read_text());out=[]
 for case in source['cases']:
  c=next(c for c in absence['cases'] if c['name']=='late-ps' and c['slots']==case['slots'])
  row=c['frames'][2];raw=packets[row['offset']:row['offset']+row['bytes']]
  old=b'\x81'+struct.pack('>hB',80,0x80)+raw;new=b'\x81'+struct.pack('>hB',90,0x80)+raw
  data=(DEST/case['file']).read_bytes();assert data.count(old)==1;data=data.replace(old,new)
  name=f"he-aac-ps-export-gaps-{case['slots']*64}-synthetic.mkv";(DEST/name).write_bytes(data)
  out.append(dict(file=name,slots=case['slots'],frame_starts=[0,2400,4800],pcm=case['pcm'],sha256=hashlib.sha256(data).hexdigest()))
 (DEST/'aac-ps-matroska-export-oracles.json').write_text(json.dumps(dict(cases=out),indent=2)+'\n');print('own PS export gap videos:',len(out))
if __name__=='__main__':main()
