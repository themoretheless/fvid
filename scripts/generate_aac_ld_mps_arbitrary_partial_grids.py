#!/usr/bin/env python3
"""Own full-band ArbitraryTree resolution changes, including return to dormant bands."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-partial-grids';GRIDS=[23,15,12,9,7,5,4];STATES=[(b,b)for b in GRIDS]+[(12,11),(15,14),(23,20),(23,21),(23,22)]
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];errors=[]
 for base in m['cases']:
  if base['name'].split('-tree')[0] not in ['plain32','shared32']:continue
  pairs=[(a,b)for a in STATES for b in STATES if a[0]!=b[0]and(a[0]!=a[1]or b[0]!=b[1])]if base['tree']==7 else[(a,(23,23)if a[0]!=23 else(12,12))for a in STATES if a[0]!=a[1]]
  for a,b in pairs:
   configs=[];references=[];frames=[];packets=[]
   for i,(bands,cutoff) in enumerate([a,b,a]):
    model=next(v for v in models if v['tree']==base['tree']and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];ssc=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:])
    if base['tree']!=7:
     bits=raw(ssc);old=raw(bytes.fromhex(base['base_ssc']));ssc=packed(bits[:20]+old[20:26]+bits[26:])
    payload=packed('100'+'0'*(base['channels']-2)+'01'+field(cutoff,5)+''.join(field(p,5)for p in range(base['channels'])));ssc+=packed(field(2,4)+length(len(payload)))+payload;configs.append(ssc.hex());cld=[(i*7+j*3+base['tree'])%31 for j in range(cutoff)];child=packed('1111'+'10001'+pcm([v for v in cld for _ in range(2)],31));spatial=body(model,i,base['slots'],'off',False)[0]+bytes([len(child)])+child;references.append(spatial.hex());r=base['frames'][i];prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(ssc,spatial),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
   old=bytes.fromhex(base['ssc']);ab=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;first=bytes.fromhex(configs[0]);asc=packed(ab[:at]+field(2,4)+length(len(first))+raw(first)+'000000');name=base['name']+f'-grid{a[0]}c{a[1]}-{b[0]}c{b[1]}-return';c=dict(base,name=name,asc=asc.hex(),ssc=configs[0],configs=configs,reference=references,grids=[a[0],b[0],a[0]],cutoffs=[a[1],b[1],a[1]],frames=frames,video=name+'.mp4',file=name+'.mkv');(OUT/c['video']).write_bytes(video_fixture([c],blob,c['channels'],write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],c['channels']));cases.append(c)
 for domain in ['plain32','shared32']:
  base=next(c for c in m['cases']if c['name']==domain+'-tree7');configs=[];references=[];packets=[];frames=[];retry=None
  for i,(bands,cuts) in enumerate([(12,[11,12]),(15,[14,14]),(12,[11,12])]):
   model=next(v for v in models if v['tree']==7 and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];ssc=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:]);payload=packed('110000'+''.join('01'+field(v,5)for v in cuts)+''.join(field(p,5)for p in range(4)));ssc+=packed(field(2,4)+length(len(payload)))+payload;configs.append(ssc.hex())
   child=packed(''.join('1111'+'10001'+pcm([((i*7+j*3+box*5)%31)for j in range(v)for _ in range(2)],31)for box,v in enumerate(cuts)));spatial=body(model,i,base['slots'],'off',False)[0]+bytes([len(child)])+child;references.append(spatial.hex());r=base['frames'][i];prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(ssc,spatial),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
   if i==1:
    oldmodel=next(v for v in models if v['tree']==7 and v['variant']==1 and v['bands']==12);child=packed(''.join('1111'+'10001'+pcm([((i*7+j*3+box*5)%31)for j in range(v)for _ in range(2)],31)for box,v in enumerate([11,12])));spatial=body(oldmodel,i,base['slots'],'off',False)[0]+bytes([len(child)])+child;retry=packed(prefix+envelope(9,1,3,header(bytes.fromhex(configs[0]),spatial),True)).hex()
  old=bytes.fromhex(base['ssc']);ab=raw(bytes.fromhex(base['asc']));markerbits=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(markerbits);assert at>=0;first=bytes.fromhex(configs[0]);asc=packed(ab[:at]+field(2,4)+length(len(first))+raw(first)+'000000');name=domain+'-mixed-cutoff-grid-refusal';c=dict(base,name=name,asc=asc.hex(),configs=configs,reference=references,grids=[12,15,12],cutoffs=[[11,12],[14,14],[11,12]],frames=frames,retry=retry,channels=4,video=name+'.mp4',file=name+'.mkv');(OUT/c['video']).write_bytes(video_fixture([c],blob,4,write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],4));errors.append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,scope='218 own covered-partial/full-grid ArbitraryTree transitions, uniform split cutoffs, plain/shared32 banks, unchanged topology; all ordered eligible states for tree7 and every partial cutoff on other trees.'),separators=(',',':'))+'\n');print('Generated',len(cases),'grid-change programmes')
if __name__=='__main__':main()
