#!/usr/bin/env python3
"""Own high-rate AAC-ELD plus128-bin ordinary LD-MPS, no external tools."""
import json,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ps_matroska_fixtures import element,number,child
from generate_aac_ld_mps_ges_fixtures import body as ges_body
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-eld-mps128';SHAPE=0
def main():
 OUT.mkdir(exist_ok=True)
 book=json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text()) if SHAPE==2 else None
 cores=generate(include_payload_bits=True,specs=[(n,rate,c,'high',0,0,True)for n in(512,480)for rate in(64000,88200,96000)for c in(1,2,6)])
 cm=json.loads(cores['manifest.json'])['cases'];am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfg=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();packets=bytearray();gold=bytearray();cases=[];errors=[];controls=[];offsets={}
 source=cores[cm[0]['video']];track=child(child(source,b'moov'),b'trak');stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl');entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC');width,height=struct.unpack_from('>HH',entry,24);sz=child(stbl,b'stsz');size=struct.unpack_from('>I',sz,4)[0]or struct.unpack_from('>I',sz,12)[0];at=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=source[at:at+size];vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
 for core in cm:
  offsets[core['name']]=len(gold);gold.extend(cores[core['name']+'.f32le']);name='core-'+core['name'];(OUT/(name+'.mp4')).write_bytes(cores[core['video']]);controls.append(dict(core,name=name,video=name+'.mp4',core_reference_offset=offsets[core['name']]))
 def build(n,rate,tree,variant,mode,bad=None):
  inputs=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6;core=next(c for c in cm if c['n']==n and c['rate']==rate and c['channels']==inputs);group=1 if n==512 else 4;slots=n*group//128
  def select(bands):return next(c for c in am if c['tree']==tree and c['variant']==variant and c['bands']==bands)
  def ssc(c,shape=SHAPE):
   data=config(c,48000,slots,cfg[c['config_offset']:c['config_offset']+c['config_bytes']],shape);return packed(frequency(rate)+raw(data)[4:])
  initial=ssc(select(23));asc=packed(field(31,5)+field(7,6)+frequency(rate)+field(core['configuration'],4)+field(n==480,1)+'0000'+field(2,4)+length(len(initial))+raw(initial)+'0000'+'00');refs=[];suffix=['']*12;previous=None;count=12//group
  for i in range(count):
   bands=(23 if i==0 else 9 if i<count-1 else 4)if mode=='grid'else 23;c=select(bands);shape=0 if SHAPE and mode=='shape-switch' and (i==1 if count==3 else 3<=i<6) else SHAPE;current=ssc(c,shape);spatial,_=ges_body(c,i,slots,'on' if bad=='run' and i==1 else 'alternating',book,shape==2,bad=='run' and i==1) if SHAPE==2 else body(c,i,slots,'alternating',shape==1)
   if bad=='trailing'and i==1:spatial+=b'\xff'
   if bad=='inband-rate'and i==1:current=packed(field(3,4)+raw(current)[4:])
   header=i==0 or mode=='repeat'or current!=previous;payload=packed('0'+field(len(current),7))+current+spatial if header else spatial;suffix[(i+1)*group-1]=envelope(9,1 if header else 0,3,payload,True);refs.append(dict(ssc=current.hex(),spatial=spatial.hex(),header=header));previous=current
  if mode=='delayed':
   indices=[(i+1)*group-1 for i in range(1,min(4,count))];joined=''.join(suffix[i]for i in indices)
   for i in indices:suffix[i]=''
   suffix[indices[-1]]=joined
  if mode=='fragmented':
   at=2*group-1;payload=bytes.fromhex(refs[1]['spatial']);half=len(payload)//2;suffix[at]=envelope(9,0,2,payload[:half],True);suffix[at+1]=envelope(9,0,1,payload[half:],True)+suffix[at+1]
  rows=[]
  for r,tail in zip(core['frames'],suffix):
   wire=r['bits'];packet=packed(wire+tail);rows.append(dict(offset=len(packets),bytes=len(packet),core_bits=len(wire)));packets.extend(packet)
  name=f'n{n}-r{rate}-tree{tree}-v{variant}-{mode}'+('-bad-'+bad if bad else'');out=dict(name=name,n=n,rate=rate,channels=inputs,configuration=core['configuration'],output_channels=[6,6,6,8,8,8,8,2][tree],tree=tree,variant=variant,mode=mode,group=group,core_count=12,slots=slots,bands=128,asc=asc.hex(),core_asc=core['asc'],ssc=initial.hex(),reference=refs,frames=rows,core_reference_offset=offsets[core['name']],samples=12*n,container_frame_samples=n,container_rate=rate,pcm_offset=0,video=name+'.mp4',file=name+'.mkv',edges=[4*n,8*n],live_prefix_packets=group+2)
  if bad:out['error']={'trailing':'trailing bytes after LD-MPS spatial frame','inband-rate':'in-band output format change','run':'reshape run exceeds frame slots'}[bad]
  (OUT/out['video']).write_bytes(video_fixture([out],packets,inputs,write=False)['data'])
  audio=element(0xe1,element(0xb5,struct.pack('>d',rate))+number(0x9f,out['output_channels']));atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,asc)+number(0x23e383,n*10**9//rate)+audio);clusters=[]
  for i,row in enumerate(rows):
   data=number(0xe7,i*n*10**6//rate)
   if i==0:data+=element(0xa3,b'\x82\0\0\x80'+picture)
   data+=element(0xa3,b'\x81\0\0\x80'+packets[row['offset']:row['offset']+row['bytes']]);clusters.append(element(0x1f43b675,data))
  header=element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2));info=element(0x1549a966,number(0x2ad7b1,1000)+element(0x4489,struct.pack('>d',12*n*10**6/rate)));(OUT/out['file']).write_bytes(header+element(0x18538067,info+element(0x1654ae6b,atrack+vtrack)+b''.join(clusters)));return out
 for n in(512,480):
  for rate in(64000,88200,96000):
   for tree in range(8):
    for variant in((0,1)if tree in(2,3,4)else(1,)):
     for mode in(('plain','repeat','grid','delayed','fragmented','shape-switch')if SHAPE else('plain','repeat','grid','delayed','fragmented')):cases.append(build(n,rate,tree,variant,mode))
   for bad in(('trailing','inband-rate','run')if SHAPE==2 else('trailing','inband-rate')):errors.append(build(n,rate,7,1,'plain',bad))
 (OUT/'packets.bin').write_bytes(packets);(OUT/'core-reference.f32le').write_bytes(gold);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],errors=errors,controls=controls,provenance='Own high-band ELD spectral packets, analytical core PCM and authored spatial parameters; ordinary128-bin MPS only, no residual/STP/GES/SBR. No private media/process/network.' if SHAPE==0 else 'Own high-rate ELD and active STP128 syntax; no private media/process/network.' if SHAPE==1 else 'Own high-rate ELD and active GES128 syntax; no private media/process/network.'),separators=(',',':'))+'\n');print(len(cases),'ordinary128-bin programmes;',len(errors),'corrupt;',len(controls),'bare ELD controls')
if __name__=='__main__':main()
