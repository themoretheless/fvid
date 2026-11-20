#!/usr/bin/env python3
"""Own resolution-change movie grammar. Offline; no process or network APIs."""
import json
from pathlib import Path
from fractions import Fraction as F
from generate_aac_ld_mps_transport_fixtures import envelope,packed,field
from generate_aac_ld_mps_inband_fixtures import header,raw
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-grid'
BANDS=[0,23,15,12,9,7,5,4]
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-inband';pm=json.loads((parent/'manifest.json').read_text())['cases'];pb=(parent/'packets.bin').read_bytes();am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();parameters=(FIX/'aac-ld-mps-assemble/packets.bin').read_bytes();blob=bytearray();cases=[]
 for base in pm:
  if base['mode']!='repeat':continue
  plain=base['name'].startswith('plain-');pairs=[(a,b)for a in range(1,8)for b in range(1,8)if a!=b]if plain else[(a,a%7+1)for a in range(1,8)]
  for a,b in pairs:
   phase=[a,b,a];configs=[];bodies=[];sources=[]
   for resolution in phase:
    c=next(c for c in am if c['tree']==base['tree']and c['variant']==1 and c['bands']==BANDS[resolution]);config=cfgs[c['config_offset']:c['config_offset']+c['config_bytes']];config=packed(field(3 if base['rate']==48000 else 6,4)+field(base['slots']-1,5)+raw(config)[9:]);f=c['frames'][0];body=parameters[f['offset']:f['offset']+f['bytes']]
    configs.append(config.hex());bodies.append(body.hex());sources.append(c['name'])
   suffixes=[envelope(9,1,3,header(bytes.fromhex(config),bytes.fromhex(body)),True)for config,body in zip(configs,bodies)]
   mode=['immediate','late','fragmented'][(a+b)%3]
   if mode=='late':suffixes=['','', ''.join(suffixes)]
   if mode=='fragmented':
    payload=header(bytes.fromhex(configs[0]),bytes.fromhex(bodies[0]));half=len(payload)//2
    suffixes=[envelope(9,1,2,payload[:half],True),envelope(9,1,1,payload[half:],True)+suffixes[1],suffixes[2]]
   frames=[]
   for f,suffix in zip(base['frames'],suffixes):
    prefix=raw(pb[f['offset']:f['offset']+f['bytes']])[:f['prefix_bits']];packet=packed(prefix+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet)
   name=base['name'].removesuffix('-repeat')+f'-res{a}-{b}-{a}'
   case={k:base[k]for k in('asc','tree','n','rate','slots','bands','channels','qmf','samples','container_rate','container_frame_samples','pcm_offset')};case.update(name=name,mode=mode,resolutions=phase,configs=configs,bodies=bodies,sources=sources,frames=frames)
   (OUT/(name+'.mp4')).write_bytes(video_fixture([case],blob,base['channels'],write=False)['data']);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Ordinary indexed23-band matrix history across all42 plain ordered resolution changes and coupled7-cycle controls; valid own headers and frame syntax, delayed/fragmented delivery and return to dormant bands.',cases=cases),separators=(',',':'))+'\n')
 # Exact rational transition oracle independent of matrix-time implementation.
 numeric=[]
 for width in (1,2,6,12):
  for a in BANDS[1:]:
   for b in BANDS[1:]:
    history=[F(0)]*(23*width);frames=[]
    for frame,bands in enumerate((a,b,a)):
     endpoint=[];last=-1;first=None
     for ps,slot in enumerate((3,7)):
      raw_values=[F(((i*7+frame*13+ps*5)%29)-14,8)for i in range(bands*width)];mask=[(band+ps+frame)%2==0 for band in range(bands)];alpha=F(slot-last,64 if ps==0 else 128)
      end=[alpha*x+(1-alpha)*history[i]if mask[i//width]else x for i,x in enumerate(raw_values)]
      if first is None:first=[history[i]+(x-history[i])/F(slot-last)for i,x in enumerate(end)]
      history[:bands*width]=end;endpoint.append(dict(slot=slot,time=64 if ps==0 else 128,mask=mask,raw=[float(x)for x in raw_values],expected=[float(x)for x in end]));last=slot
     frames.append(dict(bands=bands,endpoints=endpoint,first_slot=[float(x)for x in first]))
    numeric.append(dict(width=width,frames=frames))
 (OUT/'matrix-history.json').write_text(json.dumps(dict(scope='Exact Fraction arithmetic over indexed23-band state; all7x7 changes with return, active masks and first-slot interpolation.',cases=numeric),separators=(',',':'))+'\n')
 print(len(cases),'valid movies;',len(numeric),'exact transition cases')
if __name__=='__main__':main()
