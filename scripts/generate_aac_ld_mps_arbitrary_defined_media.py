#!/usr/bin/env python3
"""Own mixed-cutoff videos from independently classified source transition probes."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-defined'
def main():
 parent=FIX/'aac-ld-mps-arbitrary-playback';bases=json.loads((parent/'manifest.json').read_text())['cases'];source=(parent/'packets.bin').read_bytes();m=json.loads((OUT/'components.json').read_text());oracle={r['name']:r for r in json.loads((OUT/'oracle.json').read_text())['cases']};models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];errors=[]
 for base in bases:
  if base['name'].split('-tree')[0]not in ['plain32','shared32']:continue
  for component in m['cases']:
   r=oracle[component['name']];a=m['states'][component['a']];b=m['states'][component['b']]
   if base['tree']!=7 and not(r['first_error']is None and a[1][0]!=a[1][1]and b==[4,[4,4]]):continue
   configs=[];references=[];frames=[];packets=[];retry=None;channels=base['channels']+1
   def syntax(i,f):
    bands=f['bands'];cuts=f['cutoffs'];model=next(v for v in models if v['tree']==base['tree']and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];ssc=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:])
    if base['tree']!=7:
     bits=raw(ssc);old=raw(bytes.fromhex(base['base_ssc']));ssc=packed(bits[:20]+old[20:26]+bits[26:])
    nodes=component['nodes']+[[False]]*(base['channels']-3);payload=packed(''.join(''.join(str(int(v))for v in row)for row in nodes)+''.join('01'+field(v,5)for v in cuts)+''.join(field(p,5)for p in range(channels)));ssc+=packed(field(2,4)+length(len(payload)))+payload;child=packed(''.join('1111'+'10001'+pcm([((i*11+ps*7+box*3+band*5)%31)for band in range(v)for ps in range(2)],31)for box,v in enumerate(cuts)));spatial=body(model,i,base['slots'],'off',False)[0]+bytes([len(child)])+child;return ssc,spatial
   for i,f in enumerate(component['blocks']):
    ssc,spatial=syntax(i,f);configs.append(ssc.hex());references.append(spatial.hex());row=base['frames'][i];prefix=raw(source[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(ssc,spatial),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
    if r['first_error']==i:
     old=component['blocks'][i-1];ssc,spatial=syntax(i,old);retry=packed(prefix+envelope(9,1,3,header(ssc,spatial),True)).hex()
   old=bytes.fromhex(base['ssc']);ab=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;first=bytes.fromhex(configs[0]);asc=packed(ab[:at]+field(2,4)+length(len(first))+raw(first)+'000000');name=base['name']+'-'+component['name'];c=dict(base,name=name,asc=asc.hex(),configs=configs,reference=references,frames=frames,channels=channels,video=name+'.mp4',file=name+'.mkv',first_error=r['first_error'],retry=retry,component=component['name']);c.update(ssc=configs[0],grids=[f['bands']for f in component['blocks']],cutoffs=[f['cutoffs']for f in component['blocks']],source_corpus='aac-ld-mps-arbitrary-playback',source_programme=base['name']);[c.pop(k,None)for k in ['base_ssc','base_reference','extra_cld','source']];(OUT/c['video']).write_bytes(video_fixture([c],blob,channels,write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],channels));(cases if r['first_error']is None else errors).append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,scope='Own mixed-cutoff videos; source API classification is independent. Every component transition in tree7 plain/shared32; mixed initial states returning through full4 for other base trees.'),separators=(',',':'))+'\n');print('Defined media:',len(cases),'accepted,',len(errors),'undefined-history reproducers')
if __name__=='__main__':main()
