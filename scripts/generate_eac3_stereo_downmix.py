"""Owned Annex E stereo metadata and independent scalar-matrix witnesses."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-stereo-downmix'
def generate(names=None):
 files={};cases=[]
 for layout in range(8):
  for lfe in [False,True]:
   channels=COUNTS[layout]+int(lfe);family=f'l{layout}-f{int(lfe)}'
   args=dict(blocks_override=3,divisor=16)
   control,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
   files[family+'-control.ec3']=control*3
   pairs=[(c,s)for c in range(8)for s in range(8)]if layout in [5,7]else[(c,c)for c in range(8)]if layout>2 else[(0,7),(4,4),(7,0)]
   for c,s in pairs:
    if names is not None and f'{family}-c{c}-s{s}' not in names:continue
    mixes=[[(c+s)%4,c,s,7-c,7-s],[(c+s+1)%4,(c+3)%8,(s+5)%8,c,s],None]
    code=(c*8+s)%32
    lfe_codes=[code,31-code,None]if lfe and c!=s else[None]*3
    packets=[packet(layout,lfe,0,0,'mixed',None,False,stereo_mix=m,lfe_mix_code=lf,**args)[0]for m,lf in zip(mixes,lfe_codes)]
    name=f'{family}-c{c}-s{s}'
    files[name+'.ec3']=b''.join(packets);files[name+'.mkv']=movie(packets,blocks=3,channels=channels)
    cases.append(dict(name=name,control=family+'-control.ec3',layout=layout,lfe=lfe,channels=channels,rate=48000,mixes=mixes,lfe_codes=lfe_codes,frame_bytes=4096))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned EAC3 conventional spectra, independent scalar matrix oracle in tests, and neutral VP9. No private media, codec parameters, external encoder/process or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
