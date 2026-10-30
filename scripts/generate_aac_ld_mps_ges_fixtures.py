#!/usr/bin/env python3
"""Own GES component stimuli and AAC-ELD movies; explicit offline generation."""
import json,math,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed,field,length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-ges'
COUNTS=[5,5,5,7,7,4,4,2]
def config(c,rate,slots,data,shape):
 s=raw(data);at=22 if c['tree']==7 else 30;s=field(3 if rate==48000 else 6,4)+field(slots-1,5)+s[9:];s=s[:at]+field(shape,2)+s[at+2:]
 end=26 if c['tree']==7 else 35+5*int(c['tree']<5)+12*int(c['tree']in(2,3,4))
 return packed(s[:end]+('0'if shape==2 else''))
def body(c,frame,slots,pattern,book,shaped,bad_run=False):
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
   for ch,on in enumerate(flags):
    row=[]
    if on:
     while len(row)<slots:
      value=(ch+frame+len(row)//3)%5;run=min(3,slots-len(row));symbol=(value<<4)+(7 if bad_run and slots-len(row)<=3 else run-1);term=next(t for t in book if t[2]==symbol);bits+=field(term[1],term[0]);row.extend([value]*run)
    targets.append([2**(v/2-1)for v in row]if on else None)
 return packed(bits),targets if enabled and shaped else None

def main():
 OUT.mkdir(exist_ok=True);am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();parent=FIX/'aac-ld-mps-inband';bases=[c for c in json.loads((parent/'manifest.json').read_text())['cases']if c['mode']=='repeat'];pb=(parent/'packets.bin').read_bytes();sbr=FIX/'aac-eld-spatial-sbr';sm=json.loads((sbr/'manifest.json').read_text())['cases'];sb=(sbr/'packets.bin').read_bytes();book=json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text());blob=bytearray();cases=[];errors=[]
 for base in bases:
  name=base['name'].removesuffix('-repeat');plain=name.startswith('plain-')
  if plain:prefixes=[raw(pb[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']]*4
  else:
   source=next(c for c in sm if c['name']==name);prefixes=[raw(sb[r['offset']:r['offset']+r['bytes']])[:r['core_bits']+r['sbr_bits']]for r in source['frames']]
  for bands in (23,9,4):
   c=next(c for c in am if c['tree']==base['tree']and c['variant']==1 and c['bands']==bands);data=cfgs[c['config_offset']:c['config_offset']+c['config_bytes']];ssc=config(c,base['rate'],base['slots'],data,2);ordinary=config(c,base['rate'],base['slots'],data,0)
   for pattern in ('on','off-on','alternating','reconfigure'):
    if pattern=='reconfigure'and bands!=23:continue
    frames=[]
    for i,prefix in enumerate(prefixes):
     shape=0 if pattern=='reconfigure'and 3<=i<6 else 2
     selected='on'if pattern=='reconfigure'else pattern
     temporal,targets=body(c,i,base['slots'],selected,book,shape==2);normal,_=body(c,i,base['slots'],selected,book,False)
     changed=i==0 or pattern=='reconfigure'and i in(3,6);current=ssc if shape==2 else ordinary
     header=bytes([len(current)])+current+temporal if changed else temporal;packet=packed(prefix+envelope(9,1 if changed else 0,3,header,True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix),ordinary=normal.hex(),targets=targets,temporal_config=shape));blob.extend(packet)
    asc_bits=raw(bytes.fromhex(base['asc']));old=bytes.fromhex(base['configs'][0]);suffix='0010'+length(len(old))+raw(old)+'0000'+'00';at=asc_bits.rfind(suffix)
    assert at>=0 and not asc_bits[at+len(suffix):].strip('0')
    asc=packed(asc_bits[:at]+'0010'+length(len(ssc))+raw(ssc)+'0000'+'00')
    case={k:base[k]for k in('asc','tree','n','rate','slots','bands','channels','qmf','container_rate','container_frame_samples','pcm_offset')};case.update(asc=asc.hex(),name=f'{name}-bands{bands}-{pattern}',mode=pattern,parameter_bands=bands,ssc=ssc.hex(),ordinary_ssc=ordinary.hex(),frames=frames,samples=12*base['slots']*base['bands'])
    (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,base['channels'],write=False)['data']);cases.append(case)
 for good in [c for c in cases if c['name']in('plain-tree7-bands23-on','n480-c1-base-x2-crc1-bands23-on')]:
  c=next(c for c in am if c['tree']==good['tree']and c['variant']==1 and c['bands']==23)
  for bad in ('run','trailing'):
   frames=[]
   for i,row in enumerate(good['frames']):
    prefix=raw(blob[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
    temporal,_=body(c,i,good['slots'],'on',book,True,bad=='run'and i==3)
    if bad=='trailing'and i==3:temporal+=b'\xff'
    ssc=bytes.fromhex(good['ssc']);payload=bytes([len(ssc)])+ssc+temporal if i==0 else temporal
    packet=packed(prefix+envelope(9,1 if i==0 else 0,3,payload,True));frames.append(dict(row,offset=len(blob),bytes=len(packet)));blob.extend(packet)
   case=dict(good,name=good['name']+'-bad-'+bad,mode='bad-'+bad,frames=frames,error='reshape run exceeds frame slots'if bad=='run'else'trailing bytes after LD-MPS spatial frame')
   (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data']);errors.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Own all-tree GES signalling, active/off/warm-history patterns and 23/9/4 matrix resolutions for plain and coupled LD-SBR.',cases=cases,errors=errors),separators=(',',':'))+'\n')
 # Standalone input vectors preserve each programme's histories across blocks.
 samples=bytearray();components=[];cache={}
 for tree in range(8):
  inputs=[1,1,2,2,2,6,6,1][tree];outputs=[6,6,6,8,8,8,8,2][tree]
  for bands in (32,64):
   for mode in ('dense','silent','cancel','high-only','low-only'):
    blocks=[]
    for frame,slots in enumerate((1,3,8,7,4)):
     key=(inputs,outputs,bands,mode,frame);at=cache.get(key,len(samples));fresh=key not in cache;cache[key]=at;signals=[]
     for kind,chans in enumerate((inputs,outputs,outputs)):
      values=[]
      for t in range(slots):
       for ch in range(chans):
        for b in range(bands):
         active=mode!='silent'and(mode!='high-only'or b>=35)and(mode!='low-only'or b<6)
         amplitude=0.007*(1+(frame+ch)%3);phase=(frame*83+t*13+ch*7+b*3)*0.17
         z=complex(math.sin(phase),math.cos(phase*1.13))*amplitude if active else 0j
         if kind==2:z*=(-1 if mode=='cancel' else 0.4)
         values.append(z)
         if fresh:samples.extend(struct.pack('<ff',z.real,z.imag))
      signals.append(values)
     target=None if frame in(0,2)else[[2**(((ch+frame+t//3)%5)/2-1)for t in range(slots)]if(ch+frame)%3!=1 else None for ch in range(COUNTS[tree])]
     blocks.append(dict(offset=at,slots=slots,targets=target))
    components.append(dict(tree=tree,bands=bands,inputs=inputs,outputs=outputs,mode=mode,blocks=blocks))
 (OUT/'component-input.f32le').write_bytes(samples);(OUT/'component.json').write_text(json.dumps(dict(scope='Own complex QMF signals; independent GES reference is generated separately. Disabled blocks still warm envelope state.',cases=components),separators=(',',':'))+'\n')
 print(len(cases),'valid movies;',len(errors),'malformed;',len(components),'component programmes')
if __name__=='__main__':main()
