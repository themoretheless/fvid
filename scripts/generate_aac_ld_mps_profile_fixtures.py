#!/usr/bin/env python3
"""Own LD profile counterexamples,including a late residual replacement header."""
import json
from pathlib import Path
from generate_aac_ps_matroska_fixtures import child
from generate_he_aac_packet_fixtures import packed,field,frequency,video_fixture
from generate_aac_ld_mps_transport_fixtures import length,envelope
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_stp_fixtures import body
from generate_aac_ld_mps_entropy_fixtures import pcm
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';SOURCE=FIX/'aac-eld-residual64-multicore';OUT=FIX/'aac-ld-mps-profile'
def boxes(buf):
 at=0
 while at<len(buf):
  size=int.from_bytes(buf[at:at+4],'big');assert size>=8 and at+size<=len(buf);yield buf[at+4:at+8],buf[at+8:at+size];at+=size
 assert at==len(buf)
def audio_packets(data):
 tracks=[b for tag,b in boxes(child(data,b'moov'))if tag==b'trak'];track=next(t for t in tracks if child(child(t,b'mdia'),b'hdlr')[8:12]==b'soun');table=child(child(child(track,b'mdia'),b'minf'),b'stbl');sz=child(table,b'stsz');count=int.from_bytes(sz[8:12],'big');fixed=int.from_bytes(sz[4:8],'big');sizes=[fixed]*count if fixed else[int.from_bytes(sz[12+4*i:16+4*i],'big')for i in range(count)];co=child(table,b'stco');assert int.from_bytes(co[4:8],'big')==1;at=int.from_bytes(co[8:12],'big');out=[]
 for size in sizes:out.append(data[at:at+size]);at+=size
 return out
def main():
 OUT.mkdir(exist_ok=True);m=json.loads((SOURCE/'manifest.json').read_text());c=next(c for c in m['cases']if c['name']=='n512-r48000-tree7-v1-repeat');core=next(c for c in m['controls']if c['n']==512 and c['channels']==1);good_data=(SOURCE/c['control']).read_bytes();bad_data=(SOURCE/c['video']).read_bytes();good=audio_packets(good_data);bad=audio_packets(bad_data);assert len(good)==len(bad)==24;blob=bytearray()
 def rows(packets):
  out=[]
  for i,p in enumerate(packets):out.append(dict(offset=len(blob),bytes=len(p),core_bits=len(core['frames'][i]['bits'])));blob.extend(p)
  return out
 good_rows=rows(good);late_rows=rows(good[:3]+bad[3:]);cfg=packed(frequency(48000)+'00'+''.join('1'+field(b,5)if b else'0'for b in c['cutoffs']));extension=packed('0000'+length(len(cfg))+raw(cfg));ssc=bytes.fromhex(c['ssc']);assert ssc.endswith(extension);ordinary=ssc[:-len(extension)];asc=packed(field(31,5)+field(7,6)+frequency(48000)+field(1,4)+'0'+'0000'+field(2,4)+length(len(ordinary))+raw(ordinary)+'0000'+'00');case=dict(c,asc=asc.hex(),frames=late_rows)
 variants=[]
 assembly=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];spatial_config=next(a for a in assembly if a['tree']==7 and a['variant']==1 and a['bands']==23);spatial,_=body(spatial_config,1,16,'alternating',False)
 def asc_for(current):return packed(field(31,5)+field(7,6)+frequency(48000)+field(1,4)+'0'+'0000'+field(2,4)+length(len(current))+raw(current)+'0000'+'00')
 for name,kind,data in [('absent-bands',0,frequency(48000)+'00'+'0'),('zero-bands',0,frequency(48000)+'00'+'1'+field(0,5)),('downmix-zero',1,frequency(48000)+'00'+field(0,5))]:
  frame=spatial+b'\0'
  if kind==1:
   bits='011'
   for region in spatial_config['frames'][0]['parameters']:
    offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[region['kind']];values=[region['values'][ps][b]+offset for b in range(len(region['values'][0]))for ps in range(2)];bits+='1111'+'10001'+pcm(values,levels)
   frame=packed(bits+'00'+'1001'+'0000')+b'\0'
  encoded=packed(data);current=ordinary+packed(field(kind,4)+length(len(encoded))+raw(encoded));initial=name+'-initial.mp4';late_file=name+'-late.mp4';(OUT/initial).write_bytes(video_fixture([dict(c,asc=asc_for(current).hex(),frames=good_rows)],blob,1,write=False)['data']);late=good.copy();late[3]=packed(core['frames'][3]['bits']+envelope(9,1,3,header(current,frame),True));late_variant=rows(late);(OUT/late_file).write_bytes(video_fixture([dict(c,asc=asc.hex(),frames=late_variant)],blob,1,write=False)['data']);variants.append(dict(name=name,kind=kind,ssc=current.hex(),spatial_frame=frame.hex(),initial=initial,late=late_file,row=late_variant[3]))
 unknown=ordinary+packed(field(15,4)+length(1)+field(42,8));(OUT/'reserved-extension-control.mp4').write_bytes(video_fixture([dict(c,asc=asc_for(unknown).hex(),frames=good_rows)],blob,1,write=False)['data'])
 (OUT/'control.mp4').write_bytes(good_data);(OUT/'initial-residual.mp4').write_bytes(bad_data);(OUT/'late-residual.mp4').write_bytes(video_fixture([case],blob,1,write=False)['data']);(OUT/'core.f32le').write_bytes((SOURCE/c['core_gold']).read_bytes());(OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(n=512,rate=48000,channels=1,output_channels=2,ordinary_ssc=ordinary.hex(),residual_ssc=ssc.hex(),asc=asc.hex(),good=good_rows,late=late_rows,variants=variants,reserved_ssc=unknown.hex(),bad_index=3,scope='Own syntactically formed but forbidden LD-MPS residual profiles;initial and late header refusal,not accepted codec support.'),separators=(',',':'))+'\n');print('Own initial/late profile counterexamples and ordinary control generated')
if __name__=='__main__':main()
