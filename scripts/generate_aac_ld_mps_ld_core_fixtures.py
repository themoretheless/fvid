#!/usr/bin/env python3
"""Own AAC-LD plus attached LD-MPS, no encoder/process/network dependency."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_layout_fixtures import LAYOUTS,geometry,body as core_body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_ges_fixtures import body as ges_body
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-ld-core'
def main():
 OUT.mkdir(exist_ok=True);parents=json.loads((FIX/'aac-ld-layout.json').read_text())['cases'];parent_bytes=(FIX/'aac-ld-layout-packets.bin').read_bytes();am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();book=json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text());blob=bytearray();cases=[];errors=[];asc_errors=[];controls=[]
 def build(n,rate,tree,variant,mode,bad=None):
  inputs=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6;parent=next(c for c in parents if c['n']==n and c['rate']==rate and c['channels']==inputs);configuration=parent['configuration'];bins=32 if rate==24000 else 64;group=2 if n==480 and bins==64 else 1;slots=n*group//bins;shape={'stp':1,'ges':2}.get(mode,0)
  def selection(bands):return next(c for c in am if c['tree']==tree and c['variant']==variant and c['bands']==bands)
  def ssc(c):
   data=cfgs[c['config_offset']:c['config_offset']+c['config_bytes']];out=config(c,rate,slots,data,shape)
   if mode=='escaped':out+=packed(field(15,4)+length(260)+raw(bytes((i*17)%251 for i in range(260))))
   return out
  initial=ssc(selection(23));core_asc=field(23,5)+frequency(rate)+field(configuration,4)+field(n==480,1)+'00'+'00';assert packed(core_asc).hex()==parent['asc'];tail=field(0x7cc,11)+'1'+(field(len(initial),8)if len(initial)<255 else field(255,8)+field(len(initial)-255,16))+raw(initial);asc=packed(core_asc+(field(0x2b7,11)+field(5,5)+'0'if mode in('sbr-absent','ps-absent')else'')+(field(0x548,11)+'0'if mode=='ps-absent'else'')+tail)
  if bad=='asc-empty':asc=packed(core_asc+field(0x7cc,11)+'1'+field(0,8))
  if bad=='asc-short':asc=packed(core_asc+field(0x7cc,11)+'1'+field(len(initial)+10,8)+raw(initial))
  if mode=='flag-off':asc=packed(core_asc+field(0x7cc,11)+'0')
  messages=[];refs=[];previous=None
  for i in range(8//group):
   bands=(23 if i==0 else 9 if i<8//group-1 else 4)if mode=='grid'else 23;c=selection(bands);current=ssc(c)
   if shape==2:spatial,targets=ges_body(c,i,slots,'alternating',book,True)
   else:spatial,targets=body(c,i,slots,'alternating',shape==1)
   fail=(i+1)*group-1==3
   if bad=='trailing'and fail:spatial+=b'\xff'
   if bad=='inband-rate'and fail:current=bytes([((3 if rate==24000 else 6)<<4)|(current[0]&15)])+current[1:]
   header=i==0 or mode=='repeat'or current!=previous;payload=spatial
   # In-band SSC byte length is7-bit +16-bit escape, with time-align flag0.
   if header:payload=packed('0'+(field(len(current),7)if len(current)<127 else field(127,7)+field(len(current)-127,16)))+current+spatial
   messages.append(envelope(9,1 if header else 0,3,payload,True));refs.append(dict(spatial=spatial.hex(),ssc=current.hex(),header=header,targets=targets));previous=current
  suffix=['']*8
  for i,message in enumerate(messages):suffix[(i+1)*group-1]=message
  if mode=='delayed':
   indices=[(i+1)*group-1 for i in(1,2,3)];joined=''.join(suffix[i]for i in indices)
   for i in indices:suffix[i]=''
   suffix[indices[-1]]=joined
  if mode=='fragmented':
   i=1;current=bytes.fromhex(refs[i]['ssc']);spatial=bytes.fromhex(refs[i]['spatial']);payload=spatial;half=len(payload)//2;at=(i+1)*group-1;suffix[at]=envelope(9,0,2,payload[:half],True);suffix[at+1]=envelope(9,0,1,payload[half:],True)+suffix[at+1]
  if mode=='flag-off':suffix=['']*8
  rows=[];count,high=geometry(n,rate)
  for i in range(8):
   wire='';ch=0
   for element in LAYOUTS[configuration][0]:
    wire+='0000'+('0'if element==1 else'')
    for _ in range(2 if element==1 else 1):encoded,_=core_body(n,count,high,i,ch,lfe=element==3);wire+=encoded;ch+=1
   old=parent['frames'][i];assert packed(wire)==parent_bytes[old['offset']:old['offset']+old['bytes']];packet=packed(wire+suffix[i]);rows.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(wire)));blob.extend(packet)
  name=f'n{n}-r{rate}-tree{tree}-v{variant}-{mode}'+('-bad-'+bad if bad else'');out=dict(name=name,n=n,rate=rate,tree=tree,variant=variant,mode=mode,channels=inputs,configuration=configuration,bands=bins,slots=slots,group=group,asc=asc.hex(),core_asc=parent['asc'],ssc=initial.hex(),frames=rows,reference=refs,core_reference_offset=parent['reference_offset'],samples=8*n,container_frame_samples=n,container_rate=rate,pcm_offset=0)
  if mode=='padding':out['durations']=[n//2]+[n]*7;out['samples']=sum(out['durations'])
  if mode=='clock':out['container_rate']=3000;out['sample_entry_rate']=rate;out['durations']=[n*3000//rate]*8;assert n*3000%rate==0
  (OUT/(name+'.mp4')).write_bytes(video_fixture([out],blob,inputs,write=False)['data']);return out
 for n in(480,512):
  for rate in(24000,48000):
   for tree in range(8):
    for variant in((0,1)if tree in(2,3,4)else(1,)):
     for mode in('plain','repeat','delayed','fragmented','stp','ges','grid'):cases.append(build(n,rate,tree,variant,mode))
 for n,rate in((512,24000),(480,48000)):
  cases.append(build(n,rate,7,1,'escaped'))
  for mode in('padding','sbr-absent','ps-absent','clock'):cases.append(build(n,rate,7,1,mode))
  controls.append(build(n,rate,7,1,'flag-off'))
  for bad,error in(('trailing','trailing bytes after LD-MPS spatial frame'),('inband-rate','in-band output format change')):
   c=build(n,rate,7,1,'plain',bad);c['error']=error;errors.append(c)
 for bad,error in(('asc-empty','empty AAC LD spatial configuration'),('asc-short','truncated AAC LD spatial configuration')):
  c=build(512,24000,7,1,'plain',bad);c['error']=error;asc_errors.append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,asc_errors=asc_errors,provenance='Own AOT23 AAC-LD core and authored LD-MPS parameters. Frozen sparse-cosine LD PCM oracle in ../aac-ld-layout-reference.f32le. No private data/process/network. Spatial frames may span two core frames.'),separators=(',',':'))+'\n');print(len(cases),'acceptance movies;',len(controls),'flag-off;',len(errors),'runtime errors;',len(asc_errors),'ASC errors')
if __name__=='__main__':main()
