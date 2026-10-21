"""Owned missing/truncated primary slice recovery, literal nonzero PCM planes."""
import json,itertools
from pathlib import Path
from generate_avc_data_partitions import build,planes,packet,mux
from generate_avc_redundant_primary import config
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-primary-recovery'
def generate():
 files={'configuration.avcc':config()};cases=[]
 seed=packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False))
 for phase,part,good_mb,pk,rk,damage,aso in itertools.product(['reference','startup','non-reference'],[False,True],[0,1],['I','P'],['I','P'],['missing','truncated'],[False,True]):
  if phase!='reference' and (pk!='I' or rk!='I'):continue
  if phase=='startup' and part:continue # IDR cannot use data partitioning.
  name=f'partition{int(part)}-good{good_mb}-{pk}{rk}-{damage}-aso{int(aso)}'
  if phase!='reference':name=phase+'-'+name
  lost=1-good_mb;frame=0 if phase=='startup' else 1;nri=3 if phase=='startup' else (0 if phase=='non-reference' else 2)
  good=build(frame,pk,[(good_mb,'pcm')],part,first=good_mb,sid=good_mb,delta=2,nri=nri)
  bad=build(frame,pk,[(lost,'pcm')],part,first=lost,sid=lost,delta=3,nri=nri)
  bad[-1]=bad[-1][:-7]
  groups=[good,bad] if damage=='truncated' else [good]
  if aso:groups.reverse()
  primary=sum(groups,[])
  extra=build(frame,rk,[(lost,'pcm' if rk=='I' else 'zero')],part,first=lost,sid=lost,rank=1,delta=1,nri=nri)
  coded=packet(primary+extra)
  control=packet(build(frame,pk,[(good_mb,'pcm')],False,first=good_mb,delta=2,nri=nri)+build(frame,rk,[(lost,'pcm' if rk=='I' else 'zero')],False,first=lost,delta=1,nri=nri))
  tail=[packet(build(i,'P',[(0,'zero'),(1,'zero')],False,poc=(i+1)*2 if phase=='non-reference' else i*2))for i in ([2,3] if phase=='reference' else [1,2])]
  prefix=[] if phase=='startup' else [seed]
  frames=prefix+[coded]+tail;controls=prefix+[control]+tail;entries=[]
  for i,(a,b) in enumerate(zip(frames,controls)):
   fn=f'{name}-{i}.packet';cn=f'{name}-{i}-control.packet';files[fn]=a;files[cn]=b;entries.append(dict(packet=fn,control=cn))
  files[name+'-primary.packet']=packet(primary)
  files[name+'.mp4']=mux(config(),[(i,i==0,f)for i,f in enumerate(frames)],32,16,25)
  source=planes();base=bytes(sum(source,[]))
  for p,(w,bw,bh)in enumerate([(32,16,16),(16,8,8),(16,8,8)]):
   for y in range(bh):
    for x in range(w):source[p][y*w+x]+=2 if x//bw==good_mb else (1 if rk=='I' else 0)
  chosen=bytes(sum(source,[]))
  files[name+'.yuv']=base+chosen*3 if phase=='reference' else (chosen*3 if phase=='startup' else base+chosen+base*2)
  cases.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',primary=name+'-primary.packet',damage=damage,kind=pk,good_mb=good_mb,recovery_index=len(prefix),frames=entries))
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
