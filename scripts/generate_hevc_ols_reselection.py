"""Owned OLS-index-changing VPS epochs; reuse only original synthetic entropy."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration
from generate_hevc_layered_sps import vps
from generate_hevc_colour_plane_fixtures import bits,unescape,escape,packed
from generate_ffv1_gray_fixtures import element,uint
from hevc_fixture_mp4 import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-ols-reselection'
def generate():
 files={};cases=[]
 for variant in ['eos','bla','idr','illegal']:
  control='hevc-layered-vps-epoch-'+variant+'.mp4'
  cfg,groups,size=inventory(ROOT/control);params=configuration(cfg)
  fmt=dict(size=size,depth=[8+cfg[17]%8,8+cfg[18]%8],chroma=cfg[16]%4,separate=False,crop=[0]*4)
  nal,_,_=vps(fmt,base_format=fmt,extra_output=True,ordering_rows=[([4,4],0,0)]*3)
  source=bits(unescape(params[32][0][2:]))[32:128];body=bits(unescape(nal[2:]));nal=nal[:2]+escape(packed(body[:32]+source+body[128:]))
  changed=0
  for g in groups:
   for i,n in enumerate(g):
    if (n[0]>>1)&63==32:g[i]=nal;changed+=1
  assert changed==1
  parameters=[n for k in [32,33,34]for n in params[k]]
  stream=b''.join(b'\0\0\0\1'+n for n in parameters+sum(groups,[]))
  files[variant+'.mp4']=mux(stream,fmt['depth'][0],*size,chroma_format=fmt['chroma'],chroma_depth=fmt['depth'][1],access_units=groups,multilayer=True,inband_parameters=True)
  # Get exactly the muxed initial hvcC and original AU payloads.
  from generate_he_aac_packet_fixtures import boxes
  from generate_hevc_colour_plane_fixtures import child
  stbl=files[variant+'.mp4']
  for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:stbl=child(stbl,k)
  _,entry=next(boxes(child(stbl,b'stsd')[8:]));private=child(entry[78:],b'hvcC')
  packets=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups]
  track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
  payload=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+bytes([(128 if i==0 else 0)|(8 if not any(((n[0]>>1)&63)<32 for n in groups[i])else 0)])+p))for i,p in enumerate(packets))
  files[variant+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+payload)
  files[variant+'.vps']=nal
  cases.append(dict(name=variant,control=control,error='HEVC active VPS changed before sequence end'if variant=='illegal'else None))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned VPS writer changes explicit OLS masks and DPB rows; all entropy and parameters originate in repository synthetic fixtures. No private input, external codec, process or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
