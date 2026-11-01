#!/usr/bin/env python3
"""Authored EBQ plus active ArbitraryTree refusal reproducers and quant-zero controls."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-ebq'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes();blob=bytearray();cases=[];errors=[]
 for base in m['cases']:
  if base['tree']>2:continue
  for mode in range(3):
   old=bytes.fromhex(base['ssc']);ssc=bytearray(old);ssc[2]=(ssc[2]&63)|(mode<<6);ssc=bytes(ssc);ab=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;asc=packed(ab[:at]+field(2,4)+length(len(ssc))+raw(ssc)+'000000');frames=[];packets=[]
   for i,r in enumerate(base['frames']):
    prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];spatial=bytes.fromhex(base['reference'][i]);packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(ssc,spatial)if i==0 else spatial,True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet);packets.append(packet)
   name=base['name']+f'-quant{mode}';c=dict(base,name=name,ssc=ssc.hex(),asc=asc.hex(),frames=frames,quantization=mode,acceptance=mode==0,video=name+'.mp4',file=name+'.mkv')
   (OUT/c['source']).write_bytes((parent/c['source']).read_bytes());(OUT/c['video']).write_bytes(video_fixture([c],blob,c['channels'],write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,c['rate'],c['out_n'],c['channels']));(cases if mode==0 else errors).append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,scope='18 quant-zero playback controls and 36 explicit EBQ-plus-active-ArbitraryTree refusal reproducers. Syntax complete; no nonzero-EBQ spatial playback acceptance. Own synthetic source and parameters; offline explicit generation.'),separators=(',',':'))+'\n');print('Generated',len(cases),'controls and',len(errors),'refusals')
if __name__=='__main__':main()
