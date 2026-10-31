#!/usr/bin/env python3
"""Own32/128-bank residual admission reproducers,not acceptance/conformance."""
import json
from pathlib import Path
from generate_aac_eld_fixtures import generate
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_config_fixtures import TREES
from generate_aac_ld_mps_residual_native_fixtures import payload
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-residual-bank-repro'
def main():
 OUT.mkdir(exist_ok=True)
 cores=generate(include_payload_bits=True,specs=[(n,r,c,'high',0,0,True)for n in(480,512)for r in(24000,96000)for c in(1,2,6)])
 cm=json.loads(cores['manifest.json'])['cases'];am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[]
 for c in cm:
  name='core-'+c['name'];(OUT/(name+'.mp4')).write_bytes(cores[c['video']]);(OUT/(name+'.f32le')).write_bytes(cores[c['name']+'.f32le']);controls.append(dict(c,video=name+'.mp4',gold=name+'.f32le'))
 for n in(480,512):
  for rate,bins,group in((24000,32,1),(96000,128,4)):
   slots=n*group//bins
   for tree in range(8):
    inputs=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6;core=next(c for c in cm if c['n']==n and c['rate']==rate and c['channels']==inputs)
    for variant in((0,1)if tree in(2,3,4)else(1,)):
     c=next(c for c in am if c['tree']==tree and c['variant']==variant and c['bands']==23);ssc=config(c,48000,slots,cfgs[c['config_offset']:c['config_offset']+c['config_bytes']],0);ssc=packed(frequency(rate)+raw(ssc)[4:]);cutoffs=[0 if lfe else (1 if i%2==0 else 23)for i,lfe in enumerate(TREES[tree][2])]+([12]if TREES[tree][3]else[])
     ext=packed(frequency(48000)+'00'+''.join('1'+field(b,5)if b else'0'for b in cutoffs));residual_ssc=ssc+packed('0000'+length(len(ext))+raw(ext));reference=[];good=['']*(12*group);bad=['']*(12*group)
     for i in range(12):
      spatial,_=body(c,i,slots,'alternating',False);res,_=payload(c,i,slots,cutoffs);framed=(bytes([len(res)])if len(res)<255 else bytes([255])+(len(res)-255).to_bytes(2,'big'))+res
      def wrap(current,data):return envelope(9,1 if i==0 else 0,3,(bytes([len(current)])+current if i==0 else b'')+data,True)
      good[(i+1)*group-1]=wrap(ssc,spatial);bad[(i+1)*group-1]=wrap(residual_ssc,spatial+framed);reference.append(res.hex())
     def rows(suffix):
      rows=[]
      for i,tail in enumerate(suffix):
       wire=packed(core['frames'][i%12]['bits']+tail);rows.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire)
      return rows
     def asc(current):return packed(field(31,5)+field(7,6)+frequency(rate)+field(core['configuration'],4)+field(n==480,1)+'0000'+field(2,4)+length(len(current))+raw(current)+'0000'+'00')
     name=f'n{n}-r{rate}-tree{tree}-v{variant}';base=dict(name=name,n=n,rate=rate,bins=bins,bands=bins,slots=slots,group=group,tree=tree,variant=variant,channels=inputs,output_channels=[6,6,6,8,8,8,8,2][tree],samples=12*group*n,container_frame_samples=n,container_rate=rate,pcm_offset=0)
     control=dict(base,asc=asc(ssc).hex(),frames=rows(good));repro=dict(base,asc=asc(residual_ssc).hex(),frames=rows(bad));control_file=name+'-control.mp4';repro_file=name+'-residual.mp4'
     (OUT/control_file).write_bytes(video_fixture([control],blob,inputs,write=False)['data']);(OUT/repro_file).write_bytes(video_fixture([repro],blob,inputs,write=False)['data']);cases.append(dict(base,control=control_file,video=repro_file,ssc=residual_ssc.hex(),asc=repro['asc'],reference=reference))
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,scope='Syntactically valid owned residual32/128 reproducers and ordinary controls. Not proof of normative bank/phase mapping or accepted residual playback.'),separators=(',',':'))+'\n');print(len(cases),'bank refusal reproducers and paired ordinary controls;',len(controls),'bare controls')
if __name__=='__main__':main()
