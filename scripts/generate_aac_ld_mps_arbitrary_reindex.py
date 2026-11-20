#!/usr/bin/env python3
"""Authored in-band arbitrary-tree extension index changes; explicit offline generation."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-reindex'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();blob=bytearray();cases=[]
 for c in m['cases']:
  channels=c['channels'];base_out=channels-1;initial=list(range(channels));positions=[initial,initial,initial];configs=[]
  for pos in positions:
   payload=packed('100'+'0'*(base_out-1)+'00'+''.join(field(v,5)for v in pos));tree=packed(field(2,4)+length(len(payload)))+payload; ignored=bytes([0xc0]); configs.append(bytes.fromhex(c['base_ssc'])+(ignored+tree if len(configs)==1 else tree+ignored if len(configs)==2 else tree))
  assert configs[0].hex()==c['ssc'];frames=[];packets=[]
  for i,r in enumerate(c['frames']):
   prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1,3,header(configs[i],bytes.fromhex(c['reference'][i])),True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
  case=dict(c,frames=frames,configs=[v.hex()for v in configs],positions=positions)
  (OUT/case['source']).write_bytes((parent/case['source']).read_bytes());(OUT/case['video']).write_bytes(video_fixture([case],blob,channels,write=False)['data']);(OUT/case['file']).write_bytes(mkv(bytes.fromhex(case['asc']),packets,case['rate'],case['out_n'],channels));cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,scope='48 authored extension-index changes 0/1/0 using an ignored kind-12 extension, unchanged topology and output positions; three AUs MP4/Matroska. Own synthetic core/QMF and parameters.'),separators=(',',':'))+'\n');print('Generated',len(cases),'extension-index programmes')
if __name__=='__main__':main()
