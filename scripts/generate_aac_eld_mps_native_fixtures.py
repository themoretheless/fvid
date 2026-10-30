#!/usr/bin/env python3
"""Own native spatial source-association and EOF controls; offline only."""
import json,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed
from generate_he_aac_packet_fixtures import video_fixture,frequency
from generate_aac_ld_mps_transport_fixtures import field,length
from generate_aac_eld_fixtures import generate
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-mps-native'
def main():
 source=ROOT/'tests/fixtures/playback-errors/aac-eld-mps-core';OUT.mkdir(exist_ok=True)
 bodies=[(source/f'tree7-{i}.spatial').read_bytes()for i in range(3)];cores=[]
 for i in range(3):
  n,_=struct.unpack('<II',(source/f'tree7-{i}.bits').read_bytes())
  cores.append(''.join(format(v,'08b')for v in(source/f'tree7-{i}.au').read_bytes())[:n])
 half=len(bodies[0])//2;blob=bytearray();cases=[]
 for mode in('replacement','recovery','unfinished'):
  if mode=='unfinished':suffixes=[envelope(9,0,3,bodies[0],True),'','']
  else:
   suffixes=[envelope(9,0,2,bodies[0][:half],True),envelope(9,0,3,bodies[1],True),envelope(9,0,3,bodies[2],True)]
   if mode=='recovery':suffixes[1]=envelope(9,0,1,bodies[0][half:],True);suffixes[2]=envelope(9,0,3,bodies[1],True)+envelope(9,0,3,bodies[2],True)
  frames=[]
  for core,suffix in zip(cores,suffixes):
   packet=packed(core+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(core)));blob.extend(packet)
  case=dict(name=mode,asc=(source/'tree7.spatial.asc').read_bytes().hex(),channels=1,frames=frames,slots=8,bands=64,container_rate=48000,container_frame_samples=512,pcm_offset=0,samples=1536)
  (OUT/(mode+'.mp4')).write_bytes(video_fixture([case],blob,1,write=False)['data']);cases.append(case)
 # Delayed 48-kHz PCM in a distinct 90-kHz MP4 source clock.
 frames=[]
 for i,core in enumerate(cores):
  suffix=''if i<2 else''.join(envelope(9,0,3,b,True)for b in bodies)
  packet=packed(core+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(core)));blob.extend(packet)
 case=dict(name='clock90000',asc=(source/'tree7.spatial.asc').read_bytes().hex(),channels=1,frames=frames,slots=8,bands=64,container_rate=90000,sample_entry_rate=48000,container_frame_samples=960,durations=[960]*3,pcm_offset=0,samples=1536)
 (OUT/'clock90000.mp4').write_bytes(video_fixture([case],blob,1,write=False)['data']);cases.append(case)
 # Plain ELD at 24 kHz uses the 32-bin bank: both legal core lengths,
 # all spatial trees, shared ICC and active LFE bands. No LD-SBR coupling.
 spatial=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble'
 sm=json.loads((spatial/'manifest.json').read_text())['cases'];configs=(spatial/'configs.bin').read_bytes();parameters=(spatial/'packets.bin').read_bytes()
 core_files=generate(include_payload_bits=True,specs=[(n,24000,c,'base',0,0,False)for n in(480,512)for c in(1,2,6)])
 cm=json.loads(core_files['manifest.json'])['cases']
 for n in(480,512):
  for tree in range(8):
   configuration=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6
   core=next(c for c in cm if c['n']==n and c['configuration']==configuration)
   raw_cfg=next(c for c in sm if c['tree']==tree and c['variant']==1)
   cfg=configs[raw_cfg['config_offset']:raw_cfg['config_offset']+raw_cfg['config_bytes']]
   bits=''.join(field(v,8)for v in cfg);cfg=packed(field(6,4)+field(n//32-1,5)+bits[9:])
   asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(configuration,4)+('1'if n==480 else'0')+'0000'+field(2,4)+length(len(cfg))+''.join(field(v,8)for v in cfg)+'0000'+'00')
   frames=[]
   for c,f in zip(core['frames'][:3],raw_cfg['frames']):
    body=parameters[f['offset']:f['offset']+f['bytes']];packet=packed(c['bits']+envelope(9,0,3,body,True))
    frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(c['bits'])));blob.extend(packet)
   core_pcm=f'core-n{n}-c{configuration}.f32le'
   (OUT/core_pcm).write_bytes(core_files[core['name']+'.f32le'][:3*n*core['channels']*4])
   name=f'n{n}-r24000-tree{tree}'
   case=dict(name=name,tree=tree,core_pcm=core_pcm,core_asc=core['asc'],asc=asc.hex(),channels=core['channels'],frames=frames,slots=n//32,bands=32,container_rate=24000,container_frame_samples=n,pcm_offset=0,samples=3*n)
   (OUT/(name+'.mp4')).write_bytes(video_fixture([case],blob,core['channels'],write=False)['data']);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Native 32/64-bin spatial playback, independent core controls, fragment replacement rollback and strict unmatched-PCM EOF.',cases=cases),indent=2)+'\n')
if __name__=='__main__':main()
