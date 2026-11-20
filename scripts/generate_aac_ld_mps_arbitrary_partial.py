#!/usr/bin/env python3
"""Own covered partial-band acceptance and active-uncovered refusal programmes."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-partial'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();cases=[];controls=[];errors=[]
 def save(c,bands,cutoff,ssc,refs,name,acceptance,control=False,cutoffs=None,nodes_bits=None):
  channels=c['channels'];base_out=channels-1;cuts=cutoffs or[cutoff];payload=packed((nodes_bits or('100'+'0'*(base_out-1)))+''.join('01'+field(v,5)for v in cuts)+''.join(field(p,5)for p in range(channels)));new=ssc+packed(field(2,4)+length(len(payload)))+payload;ab=raw(bytes.fromhex(c['asc']));old=bytes.fromhex(c['ssc']);marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;asc=packed(ab[:at]+field(2,4)+length(len(new))+raw(new)+'000000');rows=[];packets=[];extra=[];reference=[]
  for i,r in enumerate(c['frames']):
   prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];indices=[(i*7+b*3+c['tree'])%31 for b in range(cutoff)];extra.append([facts['dequantCLD'][v]for v in indices]);child=packed(''.join('1111'+'10001'+pcm([((i*7+b*3+c['tree']+j*5)%31)for b in range(v)for ps in range(2)],31)for j,v in enumerate(cuts)));spatial=bytes.fromhex(refs[i])+bytes([len(child)])+child;reference.append(spatial.hex());packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(new,spatial)if i==0 else spatial,True));rows.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
  case=dict(c,name=name,asc=asc.hex(),ssc=new.hex(),base_ssc=ssc.hex(),base_reference=refs,reference=reference,extra_cld=extra,frames=rows,parameter_bands=bands,cutoff=cutoff,cutoffs=cuts,acceptance=acceptance,video=name+'.mp4',file=name+'.mkv')
  (OUT/case['source']).write_bytes((parent/case['source']).read_bytes());(OUT/case['video']).write_bytes(video_fixture([case],blob,channels,write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],channels));(controls if control else cases if acceptance else errors).append(case)
 for domain in ['plain32','shared32']:
  prefix=''if domain=='plain32'else'shared32-'
  for tree in range(8):
   c=next(c for c in m['cases']if c['name']==f'{domain}-tree{tree}')
   for bands in [9,12,15,23]:
    model=next(v for v in models if v['tree']==tree and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];ssc=packed(frequency(24000)+raw(config(model,48000,c['slots'],data,0))[4:]);
    if tree!=7:
     bits=raw(ssc);old=raw(bytes.fromhex(c['base_ssc']));ssc=packed(bits[:20]+old[20:26]+bits[26:])
    refs=[body(model,i,c['slots'],'off',False)[0].hex()for i in range(3)];needed=max(facts[f'QMF{bands}'][:32])+1
    save(c,bands,bands,ssc,refs,f'{prefix}tree{tree}-bands{bands}-full',True,True)
    for cutoff in range(needed,bands):save(c,bands,cutoff,ssc,refs,f'{prefix}tree{tree}-bands{bands}-covered{cutoff}',True)
    save(c,bands,needed-1,ssc,refs,f'{prefix}tree{tree}-bands{bands}-uncovered{needed-1}',False)
 for name in ['plain64-tree7','sbr-pcm128-tree7']:
  c=next(c for c in m['cases']if c['name']==name);save(c,23,22,bytes.fromhex(c['base_ssc']),c['base_reference'],name+'-uncovered22',False)
 for prefix,domain in [('', 'plain32'),('shared32-', 'shared32')]:
  base=next(c for c in controls if c['name']==prefix+'tree7-bands12-full');original=next(c for c in m['cases']if c['name']==domain+'-tree7');mixed=dict(original,channels=4)
  save(mixed,12,10,bytes.fromhex(base['base_ssc']),base['base_reference'],prefix+'tree7-bands12-mixed-uncovered10',False,cutoffs=[10,12],nodes_bits='110000')
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,scope='80 covered partial-grid acceptances,64 full-grid controls,68 explicit active-uncovered refusals; plain PCM and direct shared LD-SBR32. All8 trees; no missing active coefficient is supplied or guessed. Own synthetic data, no FFmpeg/process/network.'),separators=(',',':'))+'\n');print('Generated',len(cases),'covered acceptances,',len(controls),'controls,',len(errors),'uncovered refusals')
if __name__=='__main__':
 main()
 from generate_aac_ld_mps_arbitrary_partial_reference import authored
 authored()
