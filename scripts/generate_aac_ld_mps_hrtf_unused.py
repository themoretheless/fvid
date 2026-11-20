#!/usr/bin/env python3
"""Own unused HRTF metadata in full-output OTT-only paths, all grid pairs."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-hrtf-unused';GRIDS=[23,15,12,9,7,5,4]
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';bases=json.loads((parent/'manifest.json').read_text())['cases'];source=(parent/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[]
 def hrtf(plain,tree,bands,seed):
  bits=raw(plain);end=40 if tree<2 else 35;assert bits[34]=='0';p='00'+field(GRIDS.index(bands)+1,3)+'1'
  for ch in range(5):
   left=[(ch*7+b*3+seed)%64 for b in range(bands)];right=[(ch*11+b*5+seed*3)%64 for b in range(bands)];phase=[(ch*3+b*7+seed)%64 for b in range(bands)];p+=''.join(field(v,6)for v in left+right)+'1'+''.join(field(v,6)for v in phase)
  return packed(bits[:34]+'1'+bits[35:end]+p)
 for base in bases:
  if base['tree']not in [0,1,5,6]:continue
  tree=base['tree'];outputs=6 if tree<2 else 8;old=bytes.fromhex(base['ssc']);asc_bits=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=asc_bits.rfind(marker);assert at>=0;head=asc_bits[:at]
  for bands in GRIDS:
   model=next(c for c in models if c['tree']==tree and c['variant']==1 and c['bands']==bands);data=configs[model['config_offset']:model['config_offset']+model['config_bytes']];plain=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:]);bits=raw(plain);old_base=raw(bytes.fromhex(base['base_ssc']));plain=packed(bits[:20]+old_base[20:26]+bits[26:]);refs=[body(model,i,base['slots'],'off',False)[0]for i in range(3)]
   def save(name,sscs,all_headers,control_name,kind,hbands):
    frames=[];packets=[]
    for i,r in enumerate(base['frames']):
     prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];flag=i==0 or all_headers;packet=packed(prefix+envelope(9,1 if flag else 0,3,header(sscs[i],refs[i])if flag else refs[i],True));frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
    first=sscs[0];asc=packed(head+field(2,4)+length(len(first))+raw(first)+'000000');c=dict(name=name,asc=asc.hex(),ssc=first.hex(),configs=[v.hex()for v in sscs],reference=[v.hex()for v in refs],frames=frames,tree=tree,spatial_bands=bands,hrtf_bands=hbands,kind=kind,control=control_name,rate=base['rate'],slots=base['slots'],bands=base['bands'],bank=base['bank'],channels=outputs,out_n=base['out_n'],group=1,samples=base['samples'],container_rate=base['container_rate'],container_frame_samples=base['container_frame_samples'],pcm_offset=0,video=name+'.mp4',file=name+'.mkv');(OUT/c['video']).write_bytes(video_fixture([c],blob,outputs,write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,base['rate'],base['out_n'],outputs));return c
   stem=base['name']+f'-b{bands}';control_name=stem+'-control';controls.append(save(control_name,[plain]*3,False,None,'control',None))
   for hb in GRIDS:cases.append(save(stem+f'-hrtf{hb}',[hrtf(plain,tree,hb,0)]*3,False,control_name+'.mp4','static',[hb]*3))
   cases.append(save(stem+'-hrtf-cycle',[hrtf(plain,tree,hb,i)for i,hb in enumerate([23,4,15])],True,control_name+'.mp4','cycle',[23,4,15]))
   cases.append(save(stem+'-hrtf-on-off-on',[hrtf(plain,tree,23,1),plain,hrtf(plain,tree,4,3)],True,control_name+'.mp4','presence',[23,None,4]))
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,scope='Own HRTF metadata unused by full-output OTT-only paths0/1/5/6. All49 grid pairs per domain/tree, metadata cycles and presence switches. Full-output equivalence, not binaural HRTF rendering or general active grid conversion.'),separators=(',',':'))+'\n');print('Unused HRTF:',len(cases),'programmes;',len(controls),'controls')
if __name__=='__main__':main()
