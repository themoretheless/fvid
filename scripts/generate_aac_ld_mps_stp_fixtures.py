#!/usr/bin/env python3
"""Own STP AAC-ELD movies; explicit offline generation."""
import json,math,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed,field,length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-stp'
COUNTS=[5,5,4,6,6,4,4,2]
def config(c,rate,slots,data,shape):
 s=raw(data);at=22 if c['tree']==7 else 30;s=field(3 if rate==48000 else 6,4)+field(slots-1,5)+s[9:];s=s[:at]+field(shape,2)+s[at+2:]
 end=26 if c['tree']==7 else 35+5*int(c['tree']<5)+12*int(c['tree']in(2,3,4))
 return packed(s[:end]+('0'if shape==2 else''))
def body(c,frame,slots,pattern,shaped):
 f=c['frames'][0];bits='011'
 for r in f['parameters']:
  offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']]
  values=[r['values'][ps][b]+offset for b in range(len(r['values'][0]))for ps in range(2)]
  bits+='1111'+'10001'+pcm(values,levels)
 bits+='00'+'1001' # fixed unsmoothed first set and128-slot second-set smoothing.
 enabled=pattern=='on' or pattern=='off-on'and frame>=3 or pattern=='alternating'and frame%3!=1
 targets=[]
 if shaped:
  bits+=field(enabled,1)
  if enabled:
   flags=[pattern!='alternating'or(ch+frame)%2==0 for ch in range(COUNTS[c['tree']])];bits+=''.join(field(on,1)for on in flags)
   targets=flags
 return packed(bits),targets if enabled and shaped else None

def main():
 OUT.mkdir(exist_ok=True);am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();parent=FIX/'aac-ld-mps-inband';bases=[c for c in json.loads((parent/'manifest.json').read_text())['cases']if c['mode']=='repeat'];pb=(parent/'packets.bin').read_bytes();sbr=FIX/'aac-eld-spatial-sbr';sm=json.loads((sbr/'manifest.json').read_text())['cases'];sb=(sbr/'packets.bin').read_bytes();blob=bytearray();cases=[];errors=[]
 for base in bases:
  name=base['name'].removesuffix('-repeat');plain=name.startswith('plain-')
  if plain:prefixes=[raw(pb[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']]*4
  else:
   source=next(c for c in sm if c['name']==name);prefixes=[raw(sb[r['offset']:r['offset']+r['bytes']])[:r['core_bits']+r['sbr_bits']]for r in source['frames']]
  for bands in (23,9,4):
   c=next(c for c in am if c['tree']==base['tree']and c['variant']==1 and c['bands']==bands);data=cfgs[c['config_offset']:c['config_offset']+c['config_bytes']];ssc=config(c,base['rate'],base['slots'],data,1);ordinary=config(c,base['rate'],base['slots'],data,0)
   for pattern in ('on','off-on','alternating','reconfigure'):
    if pattern=='reconfigure'and bands!=23:continue
    frames=[]
    for i,prefix in enumerate(prefixes):
     shape=0 if pattern=='reconfigure'and 3<=i<6 else 1
     selected='on'if pattern=='reconfigure'else pattern
     temporal,targets=body(c,i,base['slots'],selected,shape==1);normal,_=body(c,i,base['slots'],selected,False)
     changed=i==0 or pattern=='reconfigure'and i in(3,6);current=ssc if shape==1 else ordinary
     header=bytes([len(current)])+current+temporal if changed else temporal;packet=packed(prefix+envelope(9,1 if changed else 0,3,header,True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix),ordinary=normal.hex(),targets=targets,temporal_config=shape));blob.extend(packet)
    asc_bits=raw(bytes.fromhex(base['asc']));old=bytes.fromhex(base['configs'][0]);suffix='0010'+length(len(old))+raw(old)+'0000'+'00';at=asc_bits.rfind(suffix)
    assert at>=0 and not asc_bits[at+len(suffix):].strip('0')
    asc=packed(asc_bits[:at]+'0010'+length(len(ssc))+raw(ssc)+'0000'+'00')
    case={k:base[k]for k in('asc','tree','n','rate','slots','bands','channels','qmf','container_rate','container_frame_samples','pcm_offset')};case.update(asc=asc.hex(),name=f'{name}-bands{bands}-{pattern}',mode=pattern,parameter_bands=bands,ssc=ssc.hex(),ordinary_ssc=ordinary.hex(),frames=frames,samples=12*base['slots']*base['bands'])
    (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,base['channels'],write=False)['data']);cases.append(case)
 for good in [c for c in cases if c['name']in('plain-tree7-bands23-on','n480-c1-base-x2-crc1-bands23-on')]:
  c=next(c for c in am if c['tree']==good['tree']and c['variant']==1 and c['bands']==23)
  for bad in ('trailing',):
   frames=[]
   for i,row in enumerate(good['frames']):
    prefix=raw(blob[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
    temporal,_=body(c,i,good['slots'],'on',True)
    if bad=='trailing'and i==3:temporal+=b'\xff'
    ssc=bytes.fromhex(good['ssc']);payload=bytes([len(ssc)])+ssc+temporal if i==0 else temporal
    packet=packed(prefix+envelope(9,1 if i==0 else 0,3,payload,True));frames.append(dict(row,offset=len(blob),bytes=len(packet)));blob.extend(packet)
   case=dict(good,name=good['name']+'-bad-'+bad,mode='bad-'+bad,frames=frames,error='trailing bytes after LD-MPS spatial frame')
   (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data']);errors.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Own all-tree STP signalling, active/off/warm-history patterns and 23/9/4 matrix resolutions for plain and coupled LD-SBR.',cases=cases,errors=errors),separators=(',',':'))+'\n')
 print(len(cases),'valid movies;',len(errors),'malformed')
if __name__=='__main__':main()
