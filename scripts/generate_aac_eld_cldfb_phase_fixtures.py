#!/usr/bin/env python3
"""Own short CLDFB canonical-phase videos; no production decoder or network."""
import json,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate,mkv
from generate_aac_eld_sbr_fixtures import analysis,reconstruct,sbr_bits
from generate_aac_eld_cldfb_fixtures import window,synthesis
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_dsp_fixtures import header
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-eld-cldfb-phase'

def main():
 OUT.mkdir(exist_ok=True)
 files=generate(True,[(n,24000,ch,'base',0,0,False) for n in [480,512] for ch in [1,2]],True,frame_count=8)
 cores=json.loads(files['manifest.json'])['cases'];c=window()
 noise_bytes=(FIX/'aac-sbr-noise-protocol.f64le').read_bytes()
 noise=[complex(*struct.unpack_from('<dd',noise_bytes,i*16)) for i in range(512)]
 _,high,low,_=tables(10,27,0,False,0,0)
 blob=bytearray();cases=[]
 for core in cores:
  n=core['n'];ch=core['channels']
  # Every ordinary spectral SF is relative to global_gain; raising all channel
  # globals by16 multiplies this own base spectrum/PCM by2^(16/4)=16 exactly.
  # There is no PNS,intensity,TNS or resilience in these base core programmes.
  pcm=[16*v for(v,)in struct.iter_unpack('<f',files[core['name']+'.f32le'])]
  prefixes=[]
  for row in core['frames']:
   assert ''.join(v['bits'] for v in row['esc'])==row['bits']
   parts=[]
   for part in row['esc']:
    b=part['bits']
    if part['category']==1:
     assert b[:8]==field(140,8);b=field(156,8)+b[8:]
    parts.append(b)
   prefixes.append(''.join(parts))
  rows=[reconstruct(analysis(pcm[k::ch],c),n//32,noise) for k in range(ch)]
  for double in [False,True]:
   bands=64 if double else 32;rate=24000*(1+double)
   asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(ch,4)+field(n==480,1)+'000'+'1'+field(double,1)+'1'+header(0,True)+'000000')
   packets=[];frames=[]
   for i,prefix in enumerate(prefixes):
    wire=packed(prefix+sbr_bits(ch,len(high)-1,len(low)-1,n//32,i,True))
    frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire);packets.append(wire)
   planar=[[v/(32768 if double else 65536) for v in synthesis(r,bands,c)] for r in rows]
   gold=b''.join(struct.pack('<'+'f'*ch,*v) for v in zip(*planar))
   name=f'n{n}-c{ch}-x{1+double}'
   case=dict(name=name,n=n,channels=ch,rate=rate,asc=asc.hex(),frames=frames,samples=8*n*(1+double),
             pcm_offset=0,video=name+'.mp4',file=name+'.mkv',golden=name+'.f32le',container_rate=rate,
             container_frame_samples=n*(1+double),slots=n//32,bands=bands,edges=[n*(1+double),3*n*(1+double)])
   (OUT/case['video']).write_bytes(video_fixture([case],blob,ch,write=False)['data'])
   (OUT/case['file']).write_bytes(mkv(asc,packets,rate,n*(1+double),ch))
   (OUT/case['golden']).write_bytes(gold);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,exports=[dict(v,name=v['name']+'-matroska',video=v['file']) for v in cases],
  scope='Own boosted nonzero480/512 mono/stereo,15/16 slots,CRC,single/dual-rate LD-SBR. Independent ISO4.6.19.4.1 canonical2n-95 direct analysis and CLDFB/HF/noise PCM. No private/ISO media/codec parameters,production decoder,FFmpeg,network or subprocess.'),separators=(',',':'))+'\n')
 print(len(cases),'canonical CLDFB phase programmes; both containers')

if __name__=='__main__':main()
