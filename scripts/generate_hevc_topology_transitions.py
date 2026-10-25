"""Owned base-only/multilayer HEVC sequence transitions, no codec processes."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration
from hevc_fixture_mp4 import mux
from generate_ffv1_gray_fixtures import element,uint
from generate_hevc_colour_plane_fixtures import child
from generate_he_aac_packet_fixtures import boxes
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-topology-transitions'
def generate():
 files={};cases=[]
 cfg,original,size=inventory(ROOT/'hevc-layered-vps-epoch-eos.mp4');params=configuration(cfg)
 basecfg,_,_=inventory(ROOT/'hevc-pcm-reference-rext8.mp4');basevps=configuration(basecfg)[32][0]
 def layer(n):return ((n[0]&1)<<5)|(n[1]>>3)
 def kind(n):return(n[0]>>1)&63
 for variant in ['base-to-multi','multi-to-base','roundtrip','illegal-base-to-multi','illegal-multi-to-base']:
  source_groups=inventory(ROOT/'hevc-layered-vps-epoch-illegal.mp4')[1]if variant.startswith('illegal')else original
  groups=[list(g)for g in source_groups];initial=dict(params)
  if variant in ['base-to-multi','roundtrip','illegal-base-to-multi']:
   initial[32]=[basevps]
   initial[34]=[n for n in params[34]if layer(n)==0]
   groups[3][1:1]=[n for n in params[34]if layer(n)!=0]
   groups[:2]=[[n for n in g if layer(n)==0]for g in groups[:2]]
  else:
   groups[3:]=[[basevps if kind(n)==32 else n for n in g if kind(n)==32 or layer(n)==0]for g in groups[3:]]
  if variant=='roundtrip':
   groups[-1].append(b'\x48\x01\x80') # base EOS before returning to base-only
   groups += [[basevps,*[n for n in original[3]if kind(n)!=32 and layer(n)==0]], [n for n in original[4]if layer(n)==0]]
  parameters=[n for k in [32,33,34]for n in initial[k]]
  stream=b''.join(b'\0\0\0\1'+n for n in parameters+sum(groups,[]))
  movie=mux(stream,8+cfg[17]%8,*size,chroma_format=cfg[16]%4,chroma_depth=8+cfg[18]%8,access_units=groups,multilayer=True,inband_parameters=True)
  files[variant+'.mp4']=movie
  stbl=movie
  for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:stbl=child(stbl,k)
  _,entry=next(boxes(child(stbl,b'stsd')[8:]));private=child(entry[78:],b'hvcC')
  packets=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups]
  track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
  payload=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+bytes([(128 if i==0 else 0)|(8 if not any(kind(n)<32 for n in groups[i])else 0)])+p))for i,p in enumerate(packets))
  files[variant+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+payload)
  indices=[i for i,g in enumerate(groups)if any(kind(n)<32 for n in g)]
  layers=[3 if any(kind(n)<32 and layer(n)==3 for n in groups[i])else 0 for i in indices]
  cases.append(dict(name=variant,samples=indices,layers=layers,oracle='hevc-pcm-reference-rext8.yuv',control='hevc-layered-vps-epoch-eos.mp4',error='HEVC active VPS changed before sequence end'if variant.startswith('illegal')else None))
 # Independent profile/segmentation axes: the same complete IDR entropy is
 # replayed across base-only -> multilayer -> base-only CVS boundaries.
 source_cases=json.loads((ROOT/'hevc-layered-reconstruction.json').read_text())['cases']
 for label in ['multislice','colour-planes']:
  c=next(c for c in source_cases if c['file']=='hevc-layered-reconstruct-'+label+'-dependent.mp4')
  cfg,original,size=inventory(ROOT/c['file']);params=configuration(cfg)
  basecfg,_,_=inventory(ROOT/c['seed']);basevps=configuration(basecfg)[32][0]
  base=[n for n in original[0]if layer(n)==0];full=list(original[0]);eos=b'\x48\x01\x80'
  groups=[base+[eos],[*params[32],*[n for n in params[34]if layer(n)!=0],*full,eos],[basevps,*base]]
  initial={32:[basevps],33:params[33],34:[n for n in params[34]if layer(n)==0]}
  parameters=[n for k in [32,33,34]for n in initial[k]]
  stream=b''.join(b'\0\0\0\1'+n for n in parameters+sum(groups,[]))
  name=label+'-roundtrip';movie=mux(stream,8+cfg[17]%8,*size,chroma_format=cfg[16]%4,chroma_depth=8+cfg[18]%8,access_units=groups,multilayer=True,inband_parameters=True)
  files[name+'.mp4']=movie
  stbl=movie
  for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:stbl=child(stbl,k)
  _,entry=next(boxes(child(stbl,b'stsd')[8:]));private=child(entry[78:],b'hvcC')
  packets=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups]
  track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
  payload=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(packets))
  files[name+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+payload)
  cases.append(dict(name=name,samples=[0,1,2],layers=[0,3,0],oracle=str(Path(c['seed']).with_suffix('.yuv')),error=None))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Repository-authored HEVC entropy, original synthetic base VPS and EOS-bound topology changes. Own MP4/EBML muxers; no private media, external process or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
