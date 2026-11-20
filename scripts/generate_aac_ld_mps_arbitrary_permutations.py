#!/usr/bin/env python3
"""Authored in-band dense position changes; explicit offline generation."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-permutations'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();blob=bytearray();cases=[]
 for c in m['cases']:
  channels=c['channels'];base_out=channels-1;initial=list(range(channels));positions=[initial,list(reversed(initial)),initial[1:]+initial[:1]];configs=[]
  for pos in positions:
   payload=packed('100'+'0'*(base_out-1)+'00'+''.join(field(v,5)for v in pos));configs.append(bytes.fromhex(c['base_ssc'])+packed(field(2,4)+length(len(payload)))+payload)
  assert configs[0].hex()==c['ssc'];frames=[];packets=[]
  for i,r in enumerate(c['frames']):
   prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(configs[i],bytes.fromhex(c['reference'][i])),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
  case=dict(c,frames=frames,configs=[v.hex()for v in configs],positions=positions)
  (OUT/case['source']).write_bytes((parent/case['source']).read_bytes());(OUT/case['video']).write_bytes(video_fixture([case],blob,channels,write=False)['data']);(OUT/case['file']).write_bytes(mkv(bytes.fromhex(case['asc']),packets,case['rate'],case['out_n'],channels));cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,scope='48 authored in-band dense output permutation changes, unchanged source topology/count, three AUs MP4/Matroska. Own synthetic core/QMF and parameters; not sparse positions or topology/grid conversion.'),separators=(',',':'))+'\n');print('Generated',len(cases),'position-change programmes')
if __name__=='__main__':main()
