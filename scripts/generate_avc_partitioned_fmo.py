"""Owned 32x32 FMO A/B/C fixtures, all seven map types and scalar pixel oracle."""
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_fmo_samples import configuration
from generate_avc_data_partitions import packet,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-fmo'
def mapping(kind,direction):
    return {0:[0,1,0,1],1:[0,1,1,0],2:[0,1,1,1],3:([0,1,0,1] if direction else [1,1,0,0]),4:([1,1,0,0] if direction else [0,0,1,1]),5:([1,0,1,0] if direction else [0,1,0,1]),6:[0,1,0,1]}[kind]
def planes():
    return [[1+(x*11+y*17+p*41)%250 for y in range(h) for x in range(w)] for p,(w,h) in enumerate([(32,32),(16,16),(16,16)])]
def write_pcm(w,mb):
    w.align()
    for p,(width,bw) in enumerate([(32,16),(16,8),(16,8)]):
        source=planes()[p]
        for y in range(bw):
            for x in range(bw):w.u(source[(mb//2*bw+y)*width+mb%2*bw+x],8)
def slice_nals(frame,addresses,kind,sid,mode,partitioned,pcm_modes=None,group_map=None):
    a=Writer();b=Writer() if partitioned else a;c=Writer() if partitioned else a
    a.ue(addresses[0]);a.ue(2 if frame==0 else 0);a.ue(0);a.u(frame,4)
    if frame==0:a.ue(0)
    a.u(frame*2,4)
    if frame:a.u(0);a.u(0)
    a.u(0)
    if frame==0:a.u(0)
    a.se(0);a.ue(1)
    if kind in [3,4,5]:a.u(2,3)
    if partitioned:
        a.ue(sid);b.ue(sid);c.ue(sid)
    counts={};used_b=used_c=False;skip=0
    for index,mb in enumerate(addresses):
        pcm=frame==0 or (mode!='coded' and (pcm_modes[mb] if pcm_modes is not None else index%2==0))
        if not pcm and mode=='skipmixed':skip+=1;continue
        if frame:a.ue(skip);skip=0
        if pcm:
            a.ue(25 if frame==0 else 30);write_pcm(b,mb);used_b=True
            for by in range(4):
                for bx in range(4):counts[(mb%2*4+bx,mb//2*4+by)]=16
        else:
            a.ue(0);a.se(0);a.se(0);a.ue(2);a.se(0);used_c=True
            for block,(bx,by) in enumerate([(0,0),(1,0),(0,1),(1,1)]):
                gx,gy=mb%2*4+bx,mb//2*4+by
                available=[counts[p] for p in [(gx-1,gy),(gx,gy-1)] if p in counts]
                nc=(sum(available)+len(available)//2)//len(available) if available else 0
                assert nc<2 or nc>=8,nc
                if block==0:
                    c.u(1,6) if nc>=8 else c.u(1,2)
                    c.u(int(group_map is not None and group_map[mb]==1));c.u(3,3);counts[(gx,gy)]=1
                else:
                    c.u(3,6) if nc>=8 else c.u(1)
                    counts[(gx,gy)]=0
            for by in range(4):
                for bx in range(4):counts.setdefault((mb%2*4+bx,mb//2*4+by),0)
    if skip:a.ue(skip)
    if not partitioned:return [a.nal(0x65 if frame==0 else 0x41)]
    return [a.nal(0x42)]+([b.nal(0x43)] if used_b else [])+([c.nal(0x44)] if used_c else [])
def generate():
    files={};cases=[]
    for kind in range(7):
     for direction in ([False,True] if kind in [3,4,5] else [False]):
      label=f'map{kind}-dir{int(direction)}';cfg=configuration(kind,direction,poc_type=0,profile=88);files[label+'.avcc']=cfg
      group_map=mapping(kind,direction);groups=[[mb for mb,g in enumerate(group_map) if g==group] for group in [0,1]]
      pcm_modes={mb:index%2==0 for group in groups for index,mb in enumerate(group)}
      seed=packet([n for sid,addresses in enumerate(groups) for n in slice_nals(0,addresses,kind,sid,'mixed',False)]);files[label+'-seed.packet']=seed
      for mode in ['coded','mixed','skipmixed']:
       for aso in [False,True]:
        for multislice in [False,True]:
         name=label+'-'+mode+'-aso'+str(int(aso))+('-multislice' if multislice else '')
         frames=[(0,True,seed)];records=[];oracle=bytearray(sum(planes(),[]))
         order=[1,0] if aso else [0,1]
         slices=[(mb,[mb]) for group in order for mb in (groups[group][::-1] if aso else groups[group])] if multislice else [(group,groups[group]) for group in order]
         for frame in [1,2]:
            parts=[slice_nals(frame,addresses,kind,sid,mode,True,pcm_modes,group_map) for sid,addresses in slices]
            actual=([n for category in [2,3,4] for group in parts for n in group if n[0]&31==category] if aso else [n for group in parts for n in group])
            control=[n for sid,addresses in slices for n in slice_nals(frame,addresses,kind,sid,mode,False,pcm_modes,group_map)]
            coded=packet(actual);files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-control.packet']=packet(control);frames.append((frame,False,coded))
            records.append(dict(packet=f'{name}-{frame}.packet',control=f'{name}-{frame}-control.packet'))
            pixels=planes()
            for group in groups:
                for index,mb in enumerate(group):
                    if mode=='coded' or mode=='mixed' and index%2:
                        for y in range(4):
                            for x,d in enumerate([4,2,-2,-4]):
                                at=(mb//2*16+y)*32+mb%2*16+x;sign=1 if group_map[mb]==0 else -1
                                pixels[0][at]=max(0,min(255,pixels[0][at]+sign*d*frame))
            oracle.extend(sum(pixels,[]))
         files[name+'.mp4']=mux(cfg,frames,32,32,25);files[name+'.yuv']=bytes(oracle)
         cases.append(dict(name=name,configuration=label+'.avcc',seed=label+'-seed.packet',file=name+'.mp4',oracle=name+'.yuv',map_type=kind,direction=direction,mapping=group_map,mode=mode,aso=aso,multislice=multislice,frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned Extended-profile FMO types0-6, literal four-MB group maps and signed group-specific scalar pixel basis. No private data/external commands/network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
