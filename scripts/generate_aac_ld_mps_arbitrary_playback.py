#!/usr/bin/env python3
"""Own additional-OTT playback sweep across eight trees and PCM/shared-QMF domains."""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-playback'
def main():
 OUT.mkdir(exist_ok=True);facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();cases=[]
 parents={n:(json.loads((FIX/n/'manifest.json').read_text()),(FIX/n/'packets.bin').read_bytes())for n in ['aac-ld-mps-downscaled','aac-ld-mps-inband','aac-ld-mps-sbr-rates']};shared=(FIX/'aac-eld-spatial-sbr/expected.f64le').read_bytes()
 for domain in ['plain32','plain64','shared32','shared64','sbr-pcm64','sbr-pcm128']:
  for tree in range(8):
   inputs=1 if tree in [0,1,7]else 2 if tree in [2,3,4]else 6;base_out=[6,6,6,8,8,8,8,2][tree]
   source=None;qmf=None
   if domain=='plain32':
    parent='aac-ld-mps-downscaled';m,packets=parents[parent];c=next(c for c in m['cases']if c['profile']==39 and c['n']==512 and c['factor']==2 and c['tree']==tree);core=next(v for v in m['controls']if v['golden']==c['core_gold']);prefixes=[raw(packets[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]for r in core['frames'][:3]];ssc=bytes.fromhex(c['ssc']);refs=c['reference'][:3];bank=32;rate=24000;sn=256;slots=8;source=(FIX/parent/c['core_gold']).read_bytes()[:3*sn*inputs*4]
   elif domain.startswith('sbr-pcm'):
    parent='aac-ld-mps-sbr-rates';m,packets=parents[parent];bank=128 if domain.endswith('128')else 64;rate=96000 if bank==128 else 48000;c=next(c for c in m['cases']if c['n']==512 and c['tree']==tree and c['rate']==rate);core=next(v for v in m['controls']if v['golden']==c['core_gold']);prefixes=[raw(packets[r['offset']:r['offset']+r['bytes']])[:r['core_bits']]for r in core['frames'][:3]];ssc=bytes.fromhex(c['ssc']);refs=c['reference'][:3];sn=c['out_n'];slots=c['slots'];source=(FIX/parent/c['core_gold']).read_bytes()[:3*sn*inputs*4]
   else:
    parent='aac-ld-mps-inband';m,packets=parents[parent]
    if domain=='plain64':c=next(c for c in m['cases']if c['name']==f'plain-tree{tree}-same');source=(FIX/f'aac-eld-mps-core/tree{tree}.pcm').read_bytes();bank=64
    else:
     bank=32 if domain=='shared32'else 64;c=next(c for c in m['cases']if c['mode']=='same'and c['n']==512 and c['tree']==tree and c['bands']==bank and c['qmf']);qmf=bytearray();at=c['qmf']['expected_offset'];stored=c['qmf']['expected_bands'];slots=c['slots']
     for frame in range(3):
      for t in range(slots):
       for ch in range(inputs):
        for b in range(bank):
         pos=at+((ch*12*slots+frame*slots+t)*stored+b)*16;qmf.extend(shared[pos:pos+16]if b<stored else bytes(16))
    rate=c['rate'];slots=c['slots'];sn=slots*bank;ssc=bytes.fromhex(c['configs'][0]);refs=[c['body']]*3;prefixes=[raw(packets[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in c['frames'][:3]]
   assert len(prefixes)==3 and len(refs)==3;assert source is None or len(source)==3*sn*inputs*4
   original_ssc=ssc
   surround=(tree+['plain32','plain64','shared32','shared64','sbr-pcm64','sbr-pcm128'].index(domain))%5;lfe=(surround+2)%5
   if tree!=7:
    bits=raw(ssc);ssc=packed(bits[:20]+field(surround,3)+field(lfe,3)+bits[26:])
   fixed=[1.0]*base_out
   if tree==0:fixed[3]=facts['lfeGainTable'][lfe];fixed[4]=fixed[5]=facts['surroundGainTable'][surround]
   elif tree in [1,2]:fixed[5]=facts['lfeGainTable'][lfe];fixed[1]=fixed[3]=facts['surroundGainTable'][surround]
   elif tree!=7:
    fixed[7]=facts['lfeGainTable'][lfe]
    for ch in [1,2,4,5]:fixed[ch]=facts['surroundGainTable'][surround]
   positions=list(range(base_out+1));data=packed('100'+'0'*(base_out-1)+'00'+''.join(field(v,5)for v in positions));new=ssc+packed(field(2,4)+length(len(data)))+data
   ab=raw(bytes.fromhex(c['asc']));marker=field(2,4)+length(len(original_ssc))+raw(original_ssc)+'000000';at=ab.rfind(marker);assert at>=0,(domain,tree);asc=packed(ab[:at]+field(2,4)+length(len(new))+raw(new)+'000000');frames=[];references=[];extra=[];name=f'{domain}-tree{tree}'
   for i,prefix in enumerate(prefixes):
    indices=[(i*7+b*3+tree)%31 for b in range(23)];extra.append([facts['dequantCLD'][v]for v in indices]);child=packed('1111'+'10001'+pcm([indices[b]for b in range(23)for ps in range(2)],31));spatial=bytes.fromhex(refs[i])+bytes([len(child)])+child;references.append(spatial.hex());packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(new,spatial)if i==0 else spatial,True));frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet)
   case=dict(name=name,asc=asc.hex(),ssc=new.hex(),base_ssc=ssc.hex(),base_reference=refs,reference=references,extra_cld=extra,frames=frames,tree=tree,bank=bank,bands=bank,rate=rate,channels=base_out+1,input_channels=inputs,out_n=sn,samples=3*sn,slots=slots,group=1,container_rate=rate,container_frame_samples=sn,pcm_offset=0,n=512,video=name+'.mp4',file=name+'.mkv',shared_qmf=qmf is not None,fixed=fixed,source=name+('.qmf.f64le'if qmf is not None else'.core.f32le'))
   (OUT/case['source']).write_bytes(qmf if qmf is not None else source);(OUT/case['video']).write_bytes(video_fixture([case],blob,base_out+1,write=False)['data']);packets_out=[blob[r['offset']:r['offset']+r['bytes']]for r in frames];(OUT/case['file']).write_bytes(mkv(asc,packets_out,rate,sn,base_out+1));cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,scope='48 authored full-band additional-OTT programmes:all eight base trees, plain32/64, shared LD-SBR32/64, PCM-coupled SBR64/128. Independent saved core PCM/QMF plus analytical extra-split composition, not an independent full foreign decoder.'),separators=(',',':'))+'\n');print('Generated',len(cases),'arbitrary-tree playback programmes')
if __name__=='__main__':main()
