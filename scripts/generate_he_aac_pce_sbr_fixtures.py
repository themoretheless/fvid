#!/usr/bin/env python3
"""Own sole-SCE/CPE PCE SBR programs, tagged packets and independent PCM pointers."""
import json, hashlib
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def program(prefix,pair,tag=3):
 bits=prefix+field(0,4)+field(1,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(0,4)+'000'+field(pair,1)+field(tag,4)
 return bits+'0'*(-len(bits)%8)+field(0,8)

def config(slots,rate,kind,pair,tag=3):
 ga=field(slots==15,1)+'00'
 prefix=(field(5,5)+frequency(24000)+'0000'+frequency(rate)+field(2,5)+ga) if kind=='explicit' else field(2,5)+frequency(24000)+'0000'+ga
 bits=program(prefix,pair,tag)
 if kind=='sync':bits+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)
 return packed(bits)

def packet(raw,pair,tag=3,pce=3):
 bits=''.join(f'{b:08b}' for b in raw)
 assert bits[:3]==('001' if pair else '000')
 return packed(program('101',pair,pce)+bits[:3]+field(tag,4)+bits[7:])

def main():
 cases=[];blob=bytearray();invalid=[]
 for channels,label,source in [(1,'mono','he-aac-sbr-packets'),(2,'pair','he-aac-sbr-stereo')]:
  manifest=json.loads((DEST/(source+'.json')).read_text());raw=(DEST/(source+'.bin' if channels==1 else 'he-aac-sbr-stereo.bin')).read_bytes()
  for slots in [15,16]:
   for bands in [32,64]:
    options=[False,True] if channels==2 else [None]
    for coupled in options:
     c=next(c for c in manifest['cases'] if c['slots']==slots and c['bands']==bands and (coupled is None or c['coupled']==coupled))
     frames=[];original=[]
     for row in c['frames']:
      src=raw[row['offset']:row['offset']+row['bytes']];original.append(src);data=packet(src,channels==2)
      frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
     for kind in ['explicit','sync','implicit']:
      if kind=='implicit' and bands==32:continue
      rate=48000 if bands==64 else 24000;setup=config(slots,rate,kind,channels==2)
      name=f'he-aac-pce-sbr-{label}-{coupled}-{kind}-{slots*64}-{rate}-synthetic.mp4'
      case=dict(slots=slots,bands=bands,frames=frames,asc=setup.hex(),pcm_offset=c['pcm_offset'],samples=c['samples'])
      video=video_fixture([case],blob,channels=channels,filename=name) if bands==64 else None
      cases.append(dict(case,channels=channels,kind=kind,video=video,output_rate=rate,wrong_asc=config(slots,rate,kind,channels==2,4).hex()))
     if slots==16 and bands==64 and coupled is not True:
      for name,tag,pce,error in [('wrong-tag',4,3,'AAC element is absent from PCE'),('changed-program',3,4,'AAC in-band PCE changed the configured layout')]:
       badframes=[]
       for n,src in enumerate(original):
        data=packet(src,channels==2,tag if n==1 else 3,pce if n==1 else 3)
        badframes.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
       case=dict(slots=slots,bands=bands,frames=badframes,asc=config(slots,48000,'explicit',channels==2).hex(),pcm_offset=0,samples=c['samples'])
       video=video_fixture([case],blob,channels=channels,filename=f'he-aac-pce-sbr-{label}-{name}-synthetic.mp4')
       invalid.append(dict(case,video=video,error=error))
 (DEST/'he-aac-pce-sbr-packets.bin').write_bytes(blob)
 (DEST/'he-aac-pce-sbr-oracles.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,sha256=hashlib.sha256(blob).hexdigest()),indent=2)+'\n')
 print('PCE SBR cases',len(cases),'refusal videos',len(invalid))
if __name__=='__main__':main()
