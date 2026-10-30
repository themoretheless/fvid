#!/usr/bin/env python3
"""Own AAC-LD/MPS and AVC Matroska, remuxed without external tools."""
import json,struct
from pathlib import Path
from generate_aac_ps_matroska_fixtures import element,number,child
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';SOURCE=FIX/'aac-ld-mps-ld-core';OUT=FIX/'aac-ld-mps-matroska'
def main():
 OUT.mkdir(exist_ok=True);m=json.loads((SOURCE/'manifest.json').read_text());blob=(SOURCE/'packets.bin').read_bytes()
 original=(SOURCE/(m['cases'][0]['name']+'.mp4')).read_bytes();track=child(child(original,b'moov'),b'trak');stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl');entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC');width,height=struct.unpack_from('>HH',entry,24);sizes=child(stbl,b'stsz');size=struct.unpack_from('>I',sizes,4)[0]or struct.unpack_from('>I',sizes,12)[0];offset=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=original[offset:offset+size]
 vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
 def movie(c):
  channels=c['channels']if c['mode']=='flag-off'else[6,6,6,8,8,8,8,2][c['tree']];audio=element(0xe1,element(0xb5,struct.pack('>d',c['rate']))+number(0x9f,channels));atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,bytes.fromhex(c['asc']))+number(0x23e383,c['n']*10**9//c['rate'])+audio);clusters=[]
  for i,row in enumerate(c['frames']):
   data=number(0xe7,i*c['n']*10**9//c['rate'])
   if i==0:data+=element(0xa3,b'\x82\0\0\x80'+picture)
   data+=element(0xa3,b'\x81\0\0\x80'+blob[row['offset']:row['offset']+row['bytes']]);clusters.append(element(0x1f43b675,data))
  header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2));info=element(0x1549a966,number(0x2ad7b1,1)+element(0x4489,struct.pack('>d',8*c['n']*10**9/c['rate'])));name=c['name']+'.mkv';(OUT/name).write_bytes(header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+b''.join(clusters)));return dict(c,file=name,output_channels=channels,mp4='../aac-ld-mps-ld-core/'+c['name']+'.mp4')
 result={key:[movie(c)for c in m[key]if c['mode']not in('clock','padding')]for key in('cases','controls','errors','asc_errors')};result['provenance']='Own AOT23/MPS and AVC source fixtures remuxed in Python; no FFmpeg/network/private media.';(OUT/'manifest.json').write_text(json.dumps(result,separators=(',',':'))+'\n');print({k:len(v)for k,v in result.items()if isinstance(v,list)})
if __name__=='__main__':main()
