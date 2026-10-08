#!/usr/bin/env python3
"""Own LC/SBR ASC without PS signalling, plus late in-band PS packets."""
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,asc,packed,field,frequency,video_fixture

def config(slots,rate,kind):
 if kind=='SBR':return asc(24000,rate,slots,'explicit',ps=False)
 return packed(field(2,5)+frequency(24000)+'0001'+field(slots==15,1)+'00')

def main():
 absence=json.loads((DEST/'aac-ps-absence-oracles.json').read_text());blob=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();cases=[]
 for c in absence['cases']:
  if c['name']!='late-ps':continue
  for kind in ['LC','SBR']:
   setup=config(c['slots'],48000,kind)
   case=dict(slots=c['slots'],bands=64,frames=c['frames'],asc=setup.hex(),pcm_offset=0,samples=c['slots']*128*3*2)
   video=video_fixture([case],blob,channels=2,filename=f"he-aac-ps-inband-{kind.lower()}-{c['slots']*64}-synthetic.mp4");video.pop('pcm_offset');video.pop('samples')
   cases.append(dict(kind=kind,slots=c['slots'],video=video,frames=c['frames'],pcm=c['pcm'],asc_core=config(c['slots'],24000,kind).hex(),detected=[False,True,True]))
 off=[]
 for slots in [15,16]:
  ga=field(slots==15,1)+'00';base=field(2,5)+frequency(24000)+'0001'+ga
  off.append(dict(slots=slots,sbr_false=packed(base+field(0x2b7,11)+field(5,5)+'0').hex(),ps_false=packed(base+field(0x2b7,11)+field(5,5)+'1'+frequency(48000)+field(0x548,11)+'0').hex()))
 (DEST/'aac-ps-inband-oracles.json').write_text(json.dumps(dict(cases=cases,explicit_false=off),indent=2)+'\n');print('own unhinted-PS videos:',len(cases))
if __name__=='__main__':main()
