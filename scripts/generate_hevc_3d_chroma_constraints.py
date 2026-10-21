"""Owned 3D Main depth chroma profile-constraint witnesses, no external codec."""
import json
from pathlib import Path
from generate_hevc_layered_sps import vps,sps,movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/hevc-3d-chroma-constraints'
def generate():
 files={};cases=[]
 for chroma in range(4):
  target=dict(chroma=chroma,separate=False,depth=[8,8],size=[128,128],crop=[0]*4)
  raw=vps(target,depth_layer=True,enhancement_profile=(8,0))[0]
  depth=sps(multilayer=True,coding_blocks=(4,5))
  stem=f'depth-chroma{chroma}'
  files[stem+'.vps']=raw;files[stem+'.sps']=depth;files[stem+'.mp4']=movie(raw,depth)
  cases.append(dict(stem=stem,chroma=chroma,valid=chroma==0))
 files['base.sps']=sps(multilayer=False,independent=True,coding_blocks=(4,5))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
