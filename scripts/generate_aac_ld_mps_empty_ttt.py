#!/usr/bin/env python3
"""Authored zero-width upper TTT region with predictive energy quantization."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-empty-ttt'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-hrtf-all-grids';old=json.loads((parent/'manifest.json').read_text());base=old['cases'][0];source=(parent/'packets.bin').read_bytes()
 prefixes=[raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']]
 bits=raw(bytes.fromhex(base['asc']));old_ssc=bytes.fromhex(base['ssc']);marker=field(2,4)+length(len(old_ssc))+raw(old_ssc)+'000000';at=bits.rfind(marker);assert at>=0;head=bits[:at]
 models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[]
 for bands in [23,15,12,9,7,5,4]:
  c=next(c for c in models if c['tree']==2 and c['variant']==4 and c['bands']==bands)
  data=cfgs[c['config_offset']:c['config_offset']+c['config_bytes']];wire=frequency(base['rate'])+raw(config(c,48000,base['slots'],data,0))[4:52];assert len(wire)==52
  for empty in ['upper','lower']:
   for quant,inactive in [(0,1)]+[(q,h)for q in [1,2]for h in [2,3,4,5]]:
    low,high,split=(1,inactive,bands)if empty=='upper'else(inactive,1,0)
    s=wire[:16]+field(quant,2)+wire[18:41]+field(low,3)+field(high,3)+field(split,5);ssc=packed(s);asc=packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000');rows=[];packets=[]
    model=c
    if empty=='lower':
     params=c['frames'][0]['parameters'];ott=[r for r in params if not r['id'].startswith('ttt-')];active=[r for r in params if r['id'].startswith('ttt-')]
     kinds=['CPC','CPC','ICC']if low<2 else['CLD','CLD']
     zero=[dict(kind=k,values=[[],[]])for k in kinds]
     model=dict(c,frames=[dict(c['frames'][0],parameters=ott+zero+active)])
    refs=[body(model,i,base['slots'],'off',False)[0]for i in range(3)]
    for i,prefix in enumerate(prefixes):
     packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(ssc,refs[i])if i==0 else refs[i],True));rows.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
    name=f'bands{bands}-quant{quant}-empty-{empty}{inactive}';case=dict(base,name=name,ssc=ssc.hex(),asc=asc.hex(),frames=rows,bands=bands,spatial_bands=bands,quant=quant,empty_region=empty,inactive_mode=inactive,reference=[r.hex()for r in refs],control_name=f'bands{bands}-quant0-empty-{empty}1',video=name+'.mp4',file=name+'.mkv')
    for key in ['hrtf','hrtf_bands','golden','core_gold','original_control']:case.pop(key,None)
    (OUT/case['video']).write_bytes(video_fixture([case],blob,6,write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,packets,base['rate'],base['out_n'],6));(controls if quant==0 else cases).append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(controls=controls,cases=cases,scope='112 authored predictive tree525 programmes with energy quantization1/2 and zero-width upper/lower TTT modes2..5;14 equivalent quant0 controls. Three AUs MP4/Matroska; no private/ISO media, process, FFmpeg or network.'),separators=(',',':'))+'\n');print('Generated',len(cases),'empty-region programmes and',len(controls),'controls')
if __name__=='__main__':main()
