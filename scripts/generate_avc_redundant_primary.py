"""Owned Extended-profile primary/redundant access units, scalar PCM oracle."""
import json,struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-redundant-primary'
def config(alternate=False,bottom=False):
    b=Writer()
    for v in [88,0,30]:b.u(v,8)
    b.ue(0);b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(1);b.ue(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67);pps=[]
    for p in range(2 if alternate else 1):
      b=Writer();b.ue(p);b.ue(0);b.u(0);b.u(int(bottom and p==1));b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(2 if p else 0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(1);pps.append(b.nal(0x68))
    return bytes([1,88,0,30,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([len(pps)])+b''.join(len(p).to_bytes(2,'big')+p for p in pps)
def planes(delta=0):return [[(x*11+y*17+p*41)%251+delta for y in range(h)for x in range(w)]for p,(w,h)in enumerate([(32,16),(16,8),(16,8)])]
def slice_nal(frame,addresses,redundant=0,pps=0,nri=None,bottom=False,bad_frame=False):
    kind='I'if frame==0 or redundant else'P';nri=(3 if frame==0 else 2)if nri is None else nri
    b=Writer();b.ue(addresses[0]);b.ue(7 if kind=='I'else 5);b.ue(pps);b.u(frame+int(bad_frame),4)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if bottom:b.se(0)
    b.ue(redundant)
    if kind=='P':b.u(0);b.u(0)
    if nri:
      if frame==0:b.u(0);b.u(0)
      else:b.u(0)
    b.se(0);b.ue(1)
    if kind=='P':b.ue(len(addresses))
    else:
      source=planes(int(redundant!=0))
      for mb in addresses:
        b.ue(25);b.align()
        for p,(w,h,bw,bh)in enumerate([(32,16,16,16),(16,8,8,8),(16,8,8,8)]):
          for y in range(bh):
            for x in range(bw):b.u(source[p][y*w+mb*bw+x],8)
    return b.nal((nri<<5)+(5 if frame==0 else 1))
def packet(nals):return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def generate():
    files={'oracle.yuv':bytes(sum(planes(),[]))*2};cases=[]
    for split in [False,True]:
      for alternate in [False,True]:
       for partial in [False,True]:
        for aso in ([False,True]if split else[False]):
          name=f'primary-split{int(split)}-pps{int(alternate)}-partial{int(partial)}-aso{int(aso)}';cfg=config(alternate);files[name+'.avcc']=cfg;frames=[];primaries=[]
          for frame in range(2):
            groups=[[0],[1]]if split else[[0,1]]
            if aso:groups=list(reversed(groups))
            main=[slice_nal(frame,a)for a in groups];redundants=[slice_nal(frame,[1]if partial else[0,1],rank,int(alternate),nri=1)for rank in ([1,2,127]if partial else[1])]
            coded=packet(main+redundants);clean=packet(main);files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-primary.packet']=clean
            frames.append((frame,frame==0,coded));primaries.append(dict(packet=f'{name}-{frame}.packet',primary=f'{name}-{frame}-primary.packet',slices=len(main),redundants=len(redundants)))
          files[name+'.mp4']=mux(cfg,frames,32,16,25);cases.append(dict(name=name,configuration=name+'.avcc',file=name+'.mp4',frames=primaries))
    bad=[];cfg=config(True);files['invalid.avcc']=cfg;main=[slice_nal(1,[0,1])]
    for name,nals,message in [
      ('wrong-frame',main+[slice_nal(1,[0,1],1,1,bad_frame=True)],'redundant slice picture identity'),
      ('wrong-reference-class',main+[slice_nal(1,[0,1],1,1,nri=0)],'redundant slice picture identity'),
      ('duplicate-primary',main+main,'overlap'),
    ]:
      files[name+'.packet']=packet(nals);bad.append(dict(file=name+'.packet',configuration='invalid.avcc',message=message))
    files['unknown-pps.avcc']=config(False);files['unknown-pps.packet']=packet(main+[slice_nal(1,[0,1],1,1)]);bad.append(dict(file='unknown-pps.packet',configuration='unknown-pps.avcc',message='unknown AVC PPS'))
    files['different-bottom.avcc']=config(True,True);files['different-bottom.packet']=packet(main+[slice_nal(1,[0,1],1,1,bottom=True)]);bad.append(dict(file='different-bottom.packet',configuration='different-bottom.avcc',message='redundant PPS POC'))
    files['redundant-only.packet']=packet([slice_nal(1,[0,1],1,1)])
    files['seed.packet']=packet([slice_nal(0,[0,1])]);files['good-p.packet']=packet(main)
    files['manifest.json']=(json.dumps(dict(provenance='Owned Extended-profile SPS/PPS, PCM I primary and P-skip primary; distinct +1 PCM redundant representations. Scalar primary samples, no tools/network/private media.',cases=cases,invalid=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
