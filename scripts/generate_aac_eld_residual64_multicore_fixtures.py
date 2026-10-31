#!/usr/bin/env python3
"""Own non-SBR ELD+residual64 multi-core acceptance candidates."""
import json,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_config_fixtures import TREES
from generate_aac_ld_mps_residual_native_fixtures import payload,control_body
from generate_aac_ld_mps_ges_fixtures import body as ges_body
from generate_aac_ps_matroska_fixtures import element,number,child
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-eld-residual64-multicore'
def main():
 OUT.mkdir(exist_ok=True)
 cores=generate(include_payload_bits=True,frame_count=24,specs=[(n,r,c,'high',0,0,True)for n in(480,512)for r in(48000,)for c in(1,2,6)])
 cm=json.loads(cores['manifest.json'])['cases'];book=json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text());am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[]
 source=cores[cm[0]['video']];track=child(child(source,b'moov'),b'trak');stbl=child(child(child(track,b'mdia'),b'minf'),b'stbl');entry=child(child(stbl,b'stsd')[8:],b'avc1');avcc=child(entry[78:],b'avcC');width,height=struct.unpack_from('>HH',entry,24);sz=child(stbl,b'stsz');size=struct.unpack_from('>I',sz,4)[0]or struct.unpack_from('>I',sz,12)[0];at=struct.unpack_from('>I',child(stbl,b'stco'),8)[0];picture=source[at:at+size];vtrack=element(0xae,number(0xd7,2)+number(0x73c5,2)+number(0x83,1)+element(0x86,b'V_MPEG4/ISO/AVC')+element(0x63a2,avcc)+element(0xe0,number(0xb0,width)+number(0xba,height)))
 def mkv(case,asc,rows):
  rate=case['rate'];n=case['n'];atrack=element(0xae,number(0xd7,1)+number(0x73c5,1)+number(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,asc)+number(0x23e383,n*10**9//rate)+element(0xe1,element(0xb5,struct.pack('>d',rate))+number(0x9f,case['output_channels'])));clusters=[]
  for i,row in enumerate(rows):
   data=number(0xe7,i*n*10**9//rate)
   if i==0:data+=element(0xa3,b'\x82\0\0\x80'+picture)
   data+=element(0xa3,b'\x81\0\0\x80'+blob[row['offset']:row['offset']+row['bytes']]);clusters.append(element(0x1f43b675,data))
  return element(0x1a45dfa3,element(0x4282,b'matroska')+number(0x4287,4)+number(0x4285,2))+element(0x18538067,element(0x1549a966,number(0x2ad7b1,1))+element(0x1654ae6b,atrack+vtrack)+b''.join(clusters))
 for c in cm:
  name='core-'+c['name'];(OUT/(name+'.mp4')).write_bytes(cores[c['video']]);(OUT/(name+'.f32le')).write_bytes(cores[c['name']+'.f32le']);controls.append(dict(c,video=name+'.mp4',gold=name+'.f32le'))
 for n in(480,512):
  for rate,bins,group in((48000,64,2),):
   slots=n*group//bins
   for tree in range(8):
    inputs=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6;core=next(c for c in cm if c['n']==n and c['rate']==rate and c['channels']==inputs)
    for variant in((0,1)if tree in(2,3,4)else(1,)):
     for mode in ('plain','repeat','delayed','fragmented','stp','ges'):
      c=next(c for c in am if c['tree']==tree and c['variant']==variant and c['bands']==23);shape={'stp':1,'ges':2}.get(mode,0);ssc=config(c,48000,slots,cfgs[c['config_offset']:c['config_offset']+c['config_bytes']],shape);ssc=packed(frequency(rate)+raw(ssc)[4:]);cutoffs=[0 if lfe else (1 if i%2==0 else 23)for i,lfe in enumerate(TREES[tree][2])]+([12]if TREES[tree][3]else[])
      ext=packed(frequency(48000)+'00'+''.join('1'+field(b,5)if b else'0'for b in cutoffs));residual_ssc=ssc+packed('0000'+length(len(ext))+raw(ext));reference=[];good=['']*(12*group);bad=['']*(12*group)
      for i in range(12):
       spatial,targets=ges_body(c,i,slots,'alternating',book,True)if shape==2 else body(c,i,slots,'alternating',shape==1);temporal=''
       if shape:
        temporal='0'if targets is None else '1'+''.join(field(bool(row)if shape==1 else row is not None,1)for row in targets)
        if shape==2 and targets is not None:
         for ch,row in enumerate(targets):
          if row is None:continue
          pos=0
          while pos<slots:
           value=(ch+i+pos//3)%5;run=min(3,slots-pos);term=next(t for t in book if t[2]==(value<<4)+run-1);temporal+=field(term[1],term[0]);pos+=run
       res,differences=payload(c,i,slots,cutoffs);framed=(bytes([len(res)])if len(res)<255 else bytes([255])+(len(res)-255).to_bytes(2,'big'))+res
       def wrap(current,data):return envelope(9,1 if i==0 or mode=='repeat' else 0,3,(bytes([len(current)])+current if i==0 or mode=='repeat' else b'')+data,True)
       good[(i+1)*group-1]=wrap(ssc,spatial);bad[(i+1)*group-1]=wrap(residual_ssc,spatial+framed);reference.append(dict(residual=res.hex(),control=control_body(c,differences,temporal).hex(),uncorrected=control_body(c,[None]*len(differences),temporal).hex()))
      if mode=='delayed':
       indices=[2*i+1 for i in(1,2,3)]
       for suffix in(good,bad):
        joined=''.join(suffix[j]for j in indices)
        for j in indices:suffix[j]=''
        suffix[indices[-1]]=joined
      if mode=='fragmented':
       # The second frame has no header; split its exact spatial payload.
       for suffix,residual in((good,False),(bad,True)):
        spatial,_=body(c,1,slots,'alternating',False);data=spatial
        if residual:
         r,_=payload(c,1,slots,cutoffs);data+=(bytes([len(r)])if len(r)<255 else bytes([255])+(len(r)-255).to_bytes(2,'big'))+r
        half=len(data)//2;suffix[3]=envelope(9,0,2,data[:half],True);suffix[4]=envelope(9,0,1,data[half:],True)+suffix[4]
      def rows(suffix):
       rows=[]
       for i,tail in enumerate(suffix):
        wire=packed(core['frames'][i%len(core['frames'])]['bits']+tail);rows.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire)
       return rows
      def asc(current):return packed(field(31,5)+field(7,6)+frequency(rate)+field(core['configuration'],4)+field(n==480,1)+'0000'+field(2,4)+length(len(current))+raw(current)+'0000'+'00')
      name=f'n{n}-r{rate}-tree{tree}-v{variant}-{mode}';base=dict(name=name,mode=mode,core_count=24,edges=[4*n,8*n,16*n],live_prefix_packets=4,n=n,rate=rate,bins=bins,bands=bins,slots=slots,group=group,tree=tree,variant=variant,channels=inputs,output_channels=[6,6,6,8,8,8,8,2][tree],samples=12*group*n,container_frame_samples=n,container_rate=rate,pcm_offset=0)
      control=dict(base,asc=asc(ssc).hex(),frames=rows(good));repro=dict(base,asc=asc(residual_ssc).hex(),frames=rows(bad));control_file=name+'-control.mp4';repro_file=name+'-residual.mp4'
      (OUT/(name+'-residual.mkv')).write_bytes(mkv(base,asc(residual_ssc),repro['frames']));(OUT/control_file).write_bytes(video_fixture([control],blob,inputs,write=False)['data']);(OUT/repro_file).write_bytes(video_fixture([repro],blob,inputs,write=False)['data']);cases.append(dict(base,control=control_file,video=repro_file,file=name+'-residual.mkv',frames=repro['frames'],ssc=residual_ssc.hex(),control_ssc=(packed(raw(residual_ssc)[:18]+'0'+raw(residual_ssc)[19:])if tree!=7 else residual_ssc).hex(),asc=repro['asc'],cutoffs=cutoffs,core_gold='core-'+core['name']+'.f32le',reference=reference))
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],controls=controls,scope='Own48kHz non-SBR ELD,paired ordinary controls and64-bin15/16-slot residual windows. Complete24-frame core scalar oracle;compositional spatial qualification,not whole-codec conformance.'),separators=(',',':'))+'\n');print(len(cases),'non-SBR residual64 movies and ordinary controls;',len(controls),'bare controls')
if __name__=='__main__':main()
