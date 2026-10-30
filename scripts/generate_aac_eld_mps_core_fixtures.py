#!/usr/bin/env python3
"""Own nonzero ELD cores followed by LD-MPS; offline fixture generation only."""
import json, struct
from pathlib import Path
from generate_aac_eld_fixtures import generate
from generate_aac_ld_mps_transport_fixtures import envelope, packed, field, length
from generate_he_aac_packet_fixtures import frequency, video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-mps-core'
def main():
 OUT.mkdir(exist_ok=True)
 cores=generate(include_payload_bits=True,specs=[(512,48000,c,'base',0,0,False)for c in(1,2,6)])
 cm=json.loads(cores['manifest.json'])['cases']
 base=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble'
 sm=json.loads((base/'manifest.json').read_text())['cases'];configs=(base/'configs.bin').read_bytes();payloads=(base/'packets.bin').read_bytes()
 cases=[]
 for tree in range(8):
  spatial=next(c for c in sm if c['tree']==tree and c['variant']==0)
  configuration=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6
  core=next(c for c in cm if c['configuration']==configuration)
  name=f'tree{tree}';ssc=bytearray(configs[spatial['config_offset']:spatial['config_offset']+spatial['config_bytes']]);ssc[0]=(3<<4)|(ssc[0]&15)
  (OUT/(name+'.ssc')).write_bytes(ssc)
  asc=packed(field(31,5)+field(7,6)+frequency(48000)+field(configuration,4)+'00000'+field(2,4)+length(len(ssc))+''.join(field(x,8)for x in ssc)+'0000'+'00')
  (OUT/(name+'.asc')).write_bytes(cores[core['name']+'.asc'])
  (OUT/(name+'.spatial.asc')).write_bytes(asc)
  (OUT/(name+'.pcm')).write_bytes(cores[core['name']+'.f32le'][:3*512*core['channels']*4])
  blob=bytearray();rows=[]
  for i,(c,s)in enumerate(zip(core['frames'][:3],spatial['frames'])):
   body=payloads[s['offset']:s['offset']+s['bytes']]
   suffix=envelope(9,0,3,body,True)
   bits=c['bits']+suffix;packet=packed(bits)
   (OUT/f'{name}-{i}.au').write_bytes(packet)
   (OUT/f'{name}-{i}.spatial').write_bytes(body)
   (OUT/f'{name}-{i}.bits').write_bytes(struct.pack('<II',len(c['bits']),len(bits)))
   rows.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet)
  movie=dict(core,asc=asc.hex(),frames=rows)
  (OUT/(name+'.mp4')).write_bytes(video_fixture([movie],blob,core['channels'],write=False)['data'])
  cases.append(dict(tree=tree,channels=core['channels'],frames=3,core_bits=[len(c['bits'])for c in core['frames'][:3]]))
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Nonzero ELD core boundary controls and native ordinary spatial playback/seek acceptance.',cases=cases),indent=2)+'\n')
if __name__=='__main__':main()
