"""Owned complete-representation AVC recovery, no external tools or private data."""
import json
from pathlib import Path
from generate_avc_data_partitions import build, planes, packet, mux
from generate_avc_redundant_primary import config
OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-redundant-recovery'
def generate():
    files={'configuration.avcc':config(True)}; cases=[]
    seed=packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False))
    files['seed.packet']=seed
    patterns=['single','large-rank','lowest','partial','late-start','truncated','alternate-pps','selected-pps','inter','disjoint','disjoint-pi','disjoint-pp']
    for partitioned in [False,True]:
      for order in ['single','split','aso']:
       def representation(rank,delta,addresses=(0,1),pps=0,kind='I'):
        groups=[[i] for i in addresses] if order!='single' else [addresses]
        if order=='aso':groups=groups[::-1]
        return sum((build(1,kind,[(i,'pcm' if kind=='I' else 'zero') for i in group],partitioned,first=group[0],sid=group[0],rank=rank,pps=pps,delta=delta) for group in groups),[])
       for pattern in patterns:
        name=f'{pattern}-partition{int(partitioned)}-{order}';rank=127 if pattern=='large-rank' else 1;pps=0;delta=1;kind='I'
        if pattern=='selected-pps':pps=1
        selected=representation(rank,1,pps=pps)
        if pattern=='lowest':selected+=representation(2,2)
        if pattern in ['partial','late-start','truncated','alternate-pps','selected-pps','inter']:
          rank=2
          low=representation(1,2,(1,) if pattern=='late-start' else (0,),pps=1 if pattern=='alternate-pps' else 0)
          if pattern=='truncated':
            low=representation(1,2);low[-1]=low[-1][:-5]
          if pattern=='inter':kind='P';delta=0
          selected=low+representation(2,delta,kind=kind,pps=pps)
        regional = pattern.startswith('disjoint') or (order!='single' and pattern in ['partial','late-start','inter','truncated'])
        low_kind='P' if pattern in ['disjoint-pi','disjoint-pp'] else 'I'
        low_delta=0 if low_kind=='P' else 2
        upper_rank=127 if pattern=='disjoint-pi' else 2
        if pattern.startswith('disjoint'):
          if pattern=='disjoint-pp':kind='P';delta=0
          selected=representation(1,low_delta,(0,),kind=low_kind)+representation(upper_rank,delta,(1,),kind=kind)
        frames=[seed,packet(selected)]
        controls=[seed,packet(build(1,kind,[(0,'pcm' if kind=='I' else 'zero'),(1,'pcm' if kind=='I' else 'zero')],False,delta=delta,pps=pps))]
        low_mb=1 if pattern=='late-start' or (pattern=='truncated' and order=='aso') else 0
        if regional:
          rank=1
          controls[1]=packet(build(1,low_kind,[(low_mb,'pcm' if low_kind=='I' else 'zero')],False,first=low_mb,delta=low_delta)+build(1,kind,[(1-low_mb,'pcm' if kind=='I' else 'zero')],False,first=1-low_mb,delta=delta))
        for frame in [2,3]:
          value=packet(build(frame,'P',[(0,'zero'),(1,'zero')],False));frames.append(value);controls.append(value)
        entries=[]
        for i,(coded,control) in enumerate(zip(frames,controls)):
          fn=f'{name}-{i}.packet';cn=f'{name}-{i}-control.packet';files[fn]=coded;files[cn]=control;entries.append(dict(packet=fn,control=cn))
        files[name+'.mp4']=mux(config(True),[(i,i==0,f) for i,f in enumerate(frames)],32,16,25)
        base=bytes(sum(planes(),[]));chosen=bytes(v+delta for v in base)
        if regional:
          mixed=planes()
          for p,(w,bw,bh) in enumerate([(32,16,16),(16,8,8),(16,8,8)]):
            for y in range(bh):
              for x in range(w):mixed[p][y*w+x]+=low_delta if x//bw==low_mb else delta
          chosen=bytes(sum(mixed,[]))
        files[name+'.yuv']=base+chosen*3
        cases.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',rank=rank,ranks=[1,upper_rank] if regional else [rank],pps=pps,frames=entries))
    for startup in [False,True]:
      for order in ['single','split','aso']:
       for rank in [1,127]:
        name=f'{"startup" if startup else "non-reference"}-{order}-rank{rank}'
        groups=[[0,1]] if order=='single' else ([[1],[0]] if order=='aso' else [[0],[1]])
        frame=0 if startup else 1
        received=packet(sum((build(frame,'I',[(i,'pcm') for i in g],False,first=g[0],rank=rank,delta=1,nri=3 if startup else 0)for g in groups),[]))
        clean=packet(build(frame,'I',[(0,'pcm'),(1,'pcm')],False,delta=1,nri=3 if startup else 0))
        frames=[] if startup else [seed];controls=list(frames)
        frames.append(received);controls.append(clean)
        for i in [1,2]:
          tail=packet(build(i,'P',[(0,'zero'),(1,'zero')],False,poc=i*2 if startup else (i+1)*2))
          frames.append(tail);controls.append(tail)
        entries=[]
        for i,(coded,control) in enumerate(zip(frames,controls)):
          fn=f'{name}-{i}.packet';cn=f'{name}-{i}-control.packet';files[fn]=coded;files[cn]=control;entries.append(dict(packet=fn,control=cn))
        files[name+'.mp4']=mux(config(True),[(i,i==0,f) for i,f in enumerate(frames)],32,16,25)
        base=bytes(sum(planes(),[]));chosen=bytes(v+1 for v in base)
        files[name+'.yuv']=chosen*3 if startup else base+chosen+base*2
        cases.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',rank=rank,pps=0,recovery_index=0 if startup else 1,frames=entries))
    invalid=[]
    for name,nals,message in [
      ('partial',build(1,'I',[(0,'pcm')],False,rank=1),'no complete coded picture'),
      ('truncated',[build(1,'I',[(0,'pcm'),(1,'pcm')],False,rank=1)[0][:-5]],'no complete coded picture'),
      ('foreign-picture',build(1,'I',[(0,'pcm')],False,rank=1)+build(2,'I',[(0,'pcm'),(1,'pcm')],False,rank=2),'identity mismatch'),
      ('reference-class',build(1,'I',[(0,'pcm')],False,rank=1)+build(1,'I',[(0,'pcm'),(1,'pcm')],False,rank=2,nri=0),'identity mismatch'),
      ('descending',build(1,'I',[(0,'pcm')],False,rank=2)+build(1,'I',[(0,'pcm'),(1,'pcm')],False,rank=1),'counter order'),
    ]:
      fn=name+'.packet';files[fn]=packet(nals);invalid.append(dict(file=fn,message=message))
    files['disjoint-ranks.packet']=packet(build(1,'I',[(0,'pcm')],False,rank=1)+build(1,'I',[(1,'pcm')],False,first=1,rank=2))
    files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=invalid),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
