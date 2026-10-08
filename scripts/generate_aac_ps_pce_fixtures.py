#!/usr/bin/env python3
"""Original mono PCE/PS programs and tagged payloads, with offline PCM references."""
import json,hashlib,struct
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_ps_matroska_fixtures import element,number,child

def program(prefix,tag=3):
 bits=prefix+field(0,4)+field(1,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(0,4)+'000'+'0'+field(tag,4)
 bits+='0'*(-len(bits)%8)+field(0,8)
 return bits

def config(slots,kind,rate=48000,tag=3):
 ga=field(slots==15,1)+'00'
 prefix=(field(2,5)+frequency(24000)+'0000'+ga) if kind=='LC' else (field(29 if kind=='PS' else 5,5)+frequency(24000)+'0000'+frequency(rate)+field(2,5)+ga)
 return packed(program(prefix,tag))

def packet(raw,sce=3,pce=3):
 bits=''.join(f'{b:08b}' for b in raw);assert bits[:3]=='000'
 return packed(program('101',pce)+bits[:3]+field(sce,4)+bits[7:])

def main():
 m=json.loads((DEST/'aac-ps-absence-oracles.json').read_text());raw=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();blob=bytearray();cases=[];bad=[]
 for c in m['cases']:
  if c['name']!='late-ps':continue
  frames=[]
  original=[]
  for row in c['frames']:
   src=raw[row['offset']:row['offset']+row['bytes']];original.append(src);p=packet(src)
   frames.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
  for kind in ['LC','SBR','PS']:
   setup=config(c['slots'],kind);v=dict(slots=c['slots'],bands=64,frames=frames,asc=setup.hex(),pcm_offset=0,samples=c['slots']*128*6)
   video=video_fixture([v],blob,channels=2,filename=f"he-aac-ps-pce-{kind.lower()}-{c['slots']*64}-synthetic.mp4");video.pop('pcm_offset');video.pop('samples')
   cases.append(dict(frames=frames,kind=kind,slots=c['slots'],video=video,asc_core=config(c['slots'],kind,24000).hex(),asc_wrong_tag=config(c['slots'],kind,48000,4).hex(),pcm=c['pcm']))
  if c['slots']==16:
   for name,args,error in [('wrong-sce-tag',dict(sce=4),'PS AAC mono tag is not configured by PCE'),('changed-program',dict(pce=4),'PS AAC in-band PCE changed the configured layout')]:
    bframes=[]
    for i,src in enumerate(original):
     p=packet(src,**(args if i==1 else {}));bframes.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
    v=dict(slots=16,bands=64,frames=bframes,asc=config(16,'PS').hex(),pcm_offset=0,samples=12288)
    video=video_fixture([v],blob,channels=2,filename=f'he-aac-ps-pce-{name}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples')
    bad.append(dict(video=video,error=error,failing_packet=1))
 source=(DEST/'avc-slice-lists-temporal.mp4').read_bytes()
 track=child(child(source,b'moov'),b'trak');stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl')
 entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC');width,height=struct.unpack_from('>HH',entry,24)
 sizes=child(stbl,b'stsz');size=struct.unpack_from('>I',sizes,4)[0] or struct.unpack_from('>I',sizes,12)[0]
 offset=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=source[offset:offset+size]
 for c in cases:
  audio=element(0xe1,element(0xb5,struct.pack('>d',48000))+number(0x9f,2 if c['kind']=='PS' else 1))
  atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,bytes.fromhex(c['video']['asc']))+number(0x23e383,(c['slots']*128*1_000_000_000+47999)//48000)+audio)
  vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
  cluster=number(0xe7,0)+element(0xa3,b'\x82'+struct.pack('>hB',0,0x80)+picture)
  for timestamp,row in zip([0,50,100],c['frames']):cluster+=element(0xa3,b'\x81'+struct.pack('>hB',timestamp,0x80)+blob[row['offset']:row['offset']+row['bytes']])
  header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))
  info=element(0x1549a966,number(0x2ad7b1,1_000_000)+element(0x4489,struct.pack('>d',150)))
  data=header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+element(0x1f43b675,cluster))
  name=c['video']['file'].replace('.mp4','.mkv');(DEST/name).write_bytes(data);c.update(matroska_file=name,matroska_sha256=hashlib.sha256(data).hexdigest())
 (DEST/'aac-ps-pce-oracles.json').write_text(json.dumps(dict(cases=cases,invalid=bad),indent=2)+'\n')
 print('original PCE/PS videos:',len(cases),'malformed:',len(bad))
if __name__=='__main__':main()
