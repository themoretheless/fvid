"""Owned PAFF SP/SI and MBAFF SP A/B/C, independent switching matrix pixels."""
import json
from pathlib import Path
import generate_avc_switching_mbaff_fixtures as mb
import generate_avc_switching_field_fixtures as field
from generate_avc_data_partitions import Writer,packet,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-interlaced-switching'
def residual(w,address,chroma=True):
    for block in range(16):w.u(1,2);w.u((block+address)%2);w.u(1)
    if chroma:
        for component in range(2):w.u(1);w.u((component+address)%2);w.u(1,2)
        for component in range(2):
            for block in range(4):w.u(1,2);w.u((block+component+address)%2);w.u(1)

def mbaff_slice(pair,fields,frame,kind,qs,coded,skip,partitioned):
    a=Writer();c=Writer() if partitioned else a;field_mode=fields[pair]
    a.ue(pair);a.ue(3);a.ue(0);a.u(frame,4);a.u(0);a.u(frame*2,4);a.u(0);a.u(0);a.u(0);a.se(0);a.u(int(kind=='secondary'));a.se(qs-26);a.ue(1)
    if partitioned:a.ue(pair);c.ue(pair)
    used=False
    if skip=='both':a.ue(2)
    else:
        for parity in [0,1]:
            if skip=='top' and parity==0:a.ue(1);continue
            if skip=='bottom' and parity==1:a.ue(1);continue
            if not (skip=='top' and parity==1):a.ue(0)
            if parity==0 or skip=='top':a.u(int(field_mode))
            a.ue(0)
            if field_mode:a.u(1)
            a.se(0);a.se(0);a.ue(12 if coded else 0)
            if coded:a.se(0);residual(c,pair*2+parity);used=True
    if not partitioned:return [a.nal(0x41)]
    return [a.nal(0x42)]+([c.nal(0x44)] if used else [])

def paff_slice(addresses,bottom,index,frame,kind,qs,coded,partitioned):
    si=kind.startswith('si');a=Writer();r=Writer() if partitioned else a
    a.ue(addresses[0]);a.ue(4 if si else 3);a.ue(0);a.u(frame,4);a.u(1);a.u(int(bottom));a.u(frame*2+index,4)
    if not si:
        a.u(0);a.u(1);a.ue(0);a.ue(1);a.ue(3) # previous same-parity field
    a.u(0);a.se(0)
    if not si:a.u(int(kind=='secondary'))
    a.se(qs-26);a.ue(1)
    if partitioned:a.ue(addresses[0]);r.ue(addresses[0])
    used=False
    if not si and not coded:a.ue(len(addresses))
    else:
        for address in addresses:
            if si:
                a.ue(1 if kind=='si-ordinary' else 0)
                for _ in range(16):a.u(1)
                a.ue(0);a.ue(0 if coded else 3)
            else:a.ue(0);a.ue(0);a.se(0);a.se(0);a.ue(12)
            if coded:a.se(0);residual(r,address);used=True
    if not partitioned:return [a.nal(0x41)]
    return [a.nal(0x42)]+([r.nal(0x43 if si else 0x44)] if used else [])

def reorder(parts,aso):
    return [n for category in [2,3,4] for group in parts for n in group if n[0]&31==category] if aso else [n for group in parts for n in group]

def generate():
    files={'mbaff.avcc':mb.configuration(),'paff.avcc':field.configuration(8,max_refs=3)};cases=[]
    for fields in [(False,False),(True,True),(False,True),(True,False)]:
     for kind in ['primary','secondary']:
      for coded in [False,True]:
       for aso in [False,True]:
        for skip in ['none','top','bottom','both']:
            name=f'mbaff-{int(fields[0])}{int(fields[1])}-{kind}-signed{int(coded)}-aso{int(aso)}-skip{skip}'
            previous=mb.source();gold=bytearray(sum(previous,[]));order=[1,0] if aso else [0,1]
            seed=packet([mb.slice_nal(pair,fields[pair],0,kind,26,False,previous,'none') for pair in order]);files[name+'-seed.packet']=seed;frames=[(0,True,seed)];records=[dict(packet=name+'-seed.packet',control=name+'-seed.packet',presentation=0,emits=True,qs=26,fields=fields)]
            for frame,qs in enumerate([0,26,51],1):
                active=[False,False] if skip=='both' else list(fields)
                parts=[mbaff_slice(pair,active,frame,kind,qs,coded,skip,True) for pair in order];actual=packet(reorder(parts,aso));control=packet([n for pair in order for n in mbaff_slice(pair,active,frame,kind,qs,coded,skip,False)])
                stem=f'{name}-{frame}';files[stem+'.packet']=actual;files[stem+'-control.packet']=control;frames.append((frame,False,actual));records.append(dict(packet=stem+'.packet',control=stem+'-control.packet',presentation=frame,emits=True,qs=qs,fields=active))
                previous=mb.reconstruct(previous,active,kind,qs,coded,skip,'both');gold.extend(sum(previous,[]))
            files[name+'.mp4']=mux(files['mbaff.avcc'],frames,32,32,25);files[name+'.yuv']=bytes(gold)
            cases.append(dict(name=name,mbaff=True,kind=kind,coded=coded,aso=aso,split=True,skip=skip,configuration='mbaff.avcc',file=name+'.mp4',oracle=name+'.yuv',frames=records))
    for kind,coded in [('primary',False),('primary',True),('secondary',False),('secondary',True),('si',False),('si',True),('si-ordinary',False)]:
     for bottom_first in [False,True]:
      for split in [False,True]:
       for aso in [False,True]:
        name=f'paff-{kind}-signed{int(coded)}-bottom{int(bottom_first)}-split{int(split)}-aso{int(aso)}';order=[True,False] if bottom_first else [False,True];history={bottom:field.planes(bottom) for bottom in order};frames=[];records=[];gold=bytearray(field.weave(history))
        for index,bottom in enumerate(order):
            seed=packet([field.pcm(bottom,index==0,bottom_first)]);stem=f'{name}-seed-{index}';files[stem+'.packet']=seed;frames.append((index,index==0,seed));records.append(dict(packet=stem+'.packet',control=stem+'.packet',presentation=0,emits=index==1,qs=26,bottom=bottom))
        for frame,qs in enumerate([0,26,51],1):
            for index,bottom in enumerate(order):
                groups=([[1],[0]] if aso else [[0],[1]]) if split else [[0,1]]
                parts=[paff_slice(group,bottom,index,frame,kind,qs,coded,True) for group in groups];actual=packet(reorder(parts,aso));control=packet([n for group in groups for n in paff_slice(group,bottom,index,frame,kind,qs,coded,False)])
                stem=f'{name}-{frame}-{index}';files[stem+'.packet']=actual;files[stem+'-control.packet']=control;frames.append((frame*2+index,False,actual));records.append(dict(packet=stem+'.packet',control=stem+'-control.packet',presentation=frame,emits=index==1,qs=qs,bottom=bottom))
                history[bottom]=([[128]*512,[128]*128,[128]*128] if kind=='si-ordinary' else field.reconstruct(history[bottom],qs,kind,coded,split))
            gold.extend(field.weave(history))
        files[name+'.mp4']=mux(files['paff.avcc'],frames,32,32,50);files[name+'.yuv']=bytes(gold)
        cases.append(dict(name=name,mbaff=False,kind=kind,coded=coded,aso=aso,split=split,skip='none',configuration='paff.avcc',file=name+'.mp4',oracle=name+'.yuv',frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned Extended PAFF SP/SI and MBAFF SP partitions with independent switching matrix oracle, QS0/26/51. No private content, external commands/network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
