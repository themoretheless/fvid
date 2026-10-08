#!/usr/bin/env python3
"""Original PS AAC/AVC Matroska with in-band discovery and negative preroll; no codec executable."""
import json, struct, hashlib
from generate_he_aac_packet_fixtures import DEST, boxes

def element(tag,data):
 width=next(w for w in range(1,9) if len(data)<(1<<(7*w))-1)
 return tag.to_bytes((tag.bit_length()+7)//8,'big')+((1<<(7*width))|len(data)).to_bytes(width,'big')+data

def number(tag,value):return element(tag,value.to_bytes(max(1,(value.bit_length()+7)//8),'big'))

def child(data,tag):return next(body for kind,body in boxes(data) if kind==tag)

def main():
 source=(DEST/'avc-slice-lists-temporal.mp4').read_bytes()
 track=child(child(source,b'moov'),b'trak');stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl')
 entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC')
 width,height=struct.unpack_from('>HH',entry,24)
 sizes=child(stbl,b'stsz');size=struct.unpack_from('>I',sizes,4)[0] or struct.unpack_from('>I',sizes,12)[0]
 offset=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=source[offset:offset+size]
 manifest=json.loads((DEST/'aac-ps-inband-oracles.json').read_text());blob=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();out=[]
 for c in manifest['cases']:
  asc=bytes.fromhex(c['video']['asc']);slots=c['slots'];pts=[-10,40,90]
  audio=element(0xe1,element(0xb5,struct.pack('>d',48000))+number(0x9f,1))
  atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,asc)+number(0x23e383,(slots*128*1_000_000_000+47999)//48000)+audio)
  vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
  cluster=number(0xe7,0)+element(0xa3,b'\x82'+struct.pack('>hB',0,0x80)+picture)
  for timestamp,row in zip(pts,c['frames']):
   packet=blob[row['offset']:row['offset']+row['bytes']]
   cluster+=element(0xa3,b'\x81'+struct.pack('>hB',timestamp,0x80)+packet)
  header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))
  info=element(0x1549a966,number(0x2ad7b1,1_000_000)+element(0x4489,struct.pack('>d',123)))
  data=header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+element(0x1f43b675,cluster))
  name=f"he-aac-ps-inband-export-{c['kind'].lower()}-{slots*64}-synthetic.mkv";(DEST/name).write_bytes(data)
  out.append(dict(file=name,slots=slots,frame_starts=[0,2400,4800],source_pts_ns=[p*1_000_000 for p in pts],first_skip_samples=480,pcm=c['pcm']['Double'],sha256=hashlib.sha256(data).hexdigest()))
 (DEST/'aac-ps-inband-matroska-export-oracles.json').write_text(json.dumps(dict(cases=out),indent=2)+'\n')
 print('original AVC/PS Matroska fixtures:',len(out))
if __name__=='__main__':main()
