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
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-grids';GRIDS=[23,15,12,9,7,5,4]
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[]
 for base in m['cases']:
  pairs=[(a,b)for a in GRIDS for b in GRIDS if a!=b]if base['tree']==7 else list(zip(GRIDS,GRIDS[1:]+GRIDS[:1]))
  for a,b in pairs:
   configs=[];references=[];frames=[];packets=[]
   for i,bands in enumerate([a,b,a]):
    model=next(v for v in models if v['tree']==base['tree']and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];ssc=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:])
    if base['tree']!=7:
     bits=raw(ssc);old=raw(bytes.fromhex(base['base_ssc']));ssc=packed(bits[:20]+old[20:26]+bits[26:])
    payload=packed('100'+'0'*(base['channels']-2)+'00'+''.join(field(p,5)for p in range(base['channels'])));ssc+=packed(field(2,4)+length(len(payload)))+payload;configs.append(ssc.hex());cld=[(i*7+j*3+base['tree'])%31 for j in range(bands)];child=packed('1111'+'10001'+pcm([v for v in cld for _ in range(2)],31));spatial=body(model,i,base['slots'],'off',False)[0]+bytes([len(child)])+child;references.append(spatial.hex());r=base['frames'][i];prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(ssc,spatial),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
   old=bytes.fromhex(base['ssc']);ab=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;first=bytes.fromhex(configs[0]);asc=packed(ab[:at]+field(2,4)+length(len(first))+raw(first)+'000000');name=base['name']+f'-grid{a}-{b}-{a}';c=dict(base,name=name,asc=asc.hex(),ssc=configs[0],configs=configs,reference=references,grids=[a,b,a],frames=frames,video=name+'.mp4',file=name+'.mkv');(OUT/c['video']).write_bytes(video_fixture([c],blob,c['channels'],write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],c['channels']));cases.append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,scope='546 own three-AU full-grid ArbitraryTree resolution changes. All42 ordered grid pairs for tree7 across six domains; seven-grid cycles for other trees. Not topology changes or partial-grid conversion.'),separators=(',',':'))+'\n');print('Generated',len(cases),'grid-change programmes')
if __name__=='__main__':main()
