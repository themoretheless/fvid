"""Owned two-reference PAFF/MBAFF B partitions and physical-coordinate oracle."""
import json
from pathlib import Path
import generate_avc_switching_mbaff_fixtures as mb
import generate_avc_switching_field_fixtures as field
from generate_avc_data_partitions import Writer,packet,mux
from generate_avc_partitioned_interlaced_switching import reorder
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-interlaced-b'
def geometry(address,component,mbaff,fields):
    side=16 if component==0 else 8
    if mbaff:
        pair,parity=divmod(address,2)
        return pair*side,parity if fields[pair] else parity*side,2 if fields[pair] else 1,side
    return address*side,0,1,side

def slice_nals(role,first,mode,spatial,mbaff,fields,split,bottom,index,partitioned):
    intra=role in ['seed','future'];is_b=role in ['b1','b2'];idr=role=='seed' and (mbaff or index==0)
    frame_num={'seed':0,'future':1,'b1':2,'b2':2,'successor':2}[role]
    poc={'seed':0,'future':6,'b1':2,'b2':4,'successor':8}[role]+(index if not mbaff else 0)
    nri=0 if is_b else (3 if idr else 2)
    a=Writer();b=Writer() if partitioned else a;c=Writer() if partitioned else a
    a.ue(first);a.ue(2 if intra else (1 if is_b else 0));a.ue(0);a.u(frame_num,4);a.u(int(not mbaff))
    if not mbaff:a.u(int(bottom))
    if idr:a.ue(0)
    a.u(poc,4)
    if is_b:a.u(int(spatial))
    if not intra:
        a.u(0)
        if mbaff:
            a.u(0)
            if is_b:a.u(0)
        else:
            if is_b or index==1:
                a.u(1);a.ue(0);a.ue(3 if is_b else 1);a.ue(3)
            else:a.u(0) # default list on first successor field detects retained B
            if is_b:a.u(1);a.ue(0);a.ue(1);a.ue(3)
    if nri:
        a.u(0)
        if idr:a.u(0)
    a.se(0);a.ue(1)
    if partitioned:a.ue(first);b.ue(first);c.ue(first)
    used_b=used_c=False
    pairs=[first] if split else [0,1]
    if is_b and mode=='skip':a.ue(len(pairs)*(2 if mbaff else 1))
    else:
      for pair in pairs:
       for parity,address in enumerate([pair*2,pair*2+1] if mbaff else [pair]):
        if not intra:a.ue(0)
        if mbaff and parity==0:a.u(int(fields[pair]))
        pcm=intra or is_b and mode=='pcm-l1' and pair==0
        if pcm:
            a.ue(25 if intra else 48);b.align();used_b=True
            source=mb.source() if mbaff else field.planes(bottom)
            for component in range(3):
                x,y,step,side=geometry(address,component,mbaff,fields);width=32 if component==0 else 16
                for row in range(side):
                    for col in range(side):b.u(source[component][(y+row*step)*width+x+col]+(4 if role=='future' else 0),8)
        else:
            selected='l1' if mode=='pcm-l1' else mode
            a.ue({'l0':1,'l1':2,'bi':3,'direct':0}[selected] if is_b else 0)
            used_lists=[0] if not is_b or selected=='l0' else ([1] if selected=='l1' else [0,1])
            if not is_b or selected!='direct':
                if mbaff and fields[pair]:
                    for _ in used_lists:a.u(1)
                for _ in used_lists:a.se(0);a.se(0)
            a.ue(2 if is_b else 0)
            if is_b:
                a.se(0);used_c=True;left_pcm=mode=='pcm-l1' and pair==1 and not split
                c.u(1,6) if left_pcm else c.u(1,2)
                c.u(int(pair==1));c.u(3,3);c.u(1)
                c.u(3,6) if left_pcm else c.u(1)
                c.u(1)
    if not partitioned:return [a.nal((nri<<5)+(5 if idr else 1))]
    return [a.nal((nri<<5)+2)]+([b.nal((nri<<5)+3)] if used_b else [])+([c.nal((nri<<5)+4)] if used_c else [])

def expected_field(source,mode,mbaff,fields):
    result=[p[:] for p in source]
    for address in range(4 if mbaff else 2):
        pair=address//2 if mbaff else address;pcm=mode=='pcm-l1' and pair==0;selected='l1' if mode=='pcm-l1' else mode
        shift=0 if pcm else {'l0':0,'l1':4,'bi':2,'direct':2,'skip':2}[selected]
        for component in range(3):
            x,y,step,side=geometry(address,component,mbaff,fields);width=32 if component==0 else 16
            for row in range(side):
                for col in range(side):result[component][(y+row*step)*width+x+col]+=shift
        if not pcm and mode!='skip':
            x,y,step,_=geometry(address,0,mbaff,fields);vertical=not mbaff or fields[pair]
            for row in range(4):
                for col in range(4):
                    at=(y+row*step)*32+x+col;delta=[4,2,-2,-4][row if vertical else col]*(1 if pair==0 else -1)
                    result[0][at]=max(0,min(255,result[0][at]+delta))
    return result

def generate():
    files={'mbaff.avcc':mb.configuration(),'paff.avcc':field.configuration(8,max_refs=3)};cases=[]
    for mbaff in [True,False]:
     for fields in ([(False,False),(True,True),(False,True),(True,False)] if mbaff else [(False,False),(True,True)]):
      for split in ([True] if mbaff else [False,True]):
       for aso in [False,True]:
        for mode in ['l0','l1','bi','direct','skip','pcm-l1']:
         for spatial in ([False,True] if mode in ['direct','skip'] else [True]):
            name=('mbaff' if mbaff else 'paff')+f'-{int(fields[0])}{int(fields[1])}-split{int(split)}-aso{int(aso)}-{mode}-spatial{int(spatial)}'
            cfgname='mbaff.avcc' if mbaff else 'paff.avcc';frames=[];records=[];base=mb.source() if mbaff else {bottom:field.planes(bottom) for bottom in [False,True]};active=[False,False] if mbaff and mode=='skip' else list(fields)
            if mbaff:
                raw=bytes(sum(base,[]));later=bytes(v+4 for v in raw);braw=bytes(sum(expected_field(base,mode,True,active),[]))
            else:
                raw=field.weave(base);later=bytes(v+4 for v in raw);braw=field.weave({bottom:expected_field(base[bottom],mode,False,active) for bottom in [False,True]})
            golden=raw+braw*2+later*2
            for role,presentation in [('seed',0),('future',3),('b1',1),('b2',2),('successor',4)]:
                topology=list(fields) if role in ['seed','future'] else active
                order=[False] if mbaff else ([True,False] if fields[0] else [False,True])
                for index,bottom in enumerate(order):
                    groups=([1,0] if aso else [0,1]) if split else [0]
                    part=role in ['b1','b2'];parts=[slice_nals(role,first,mode,spatial,mbaff,topology,split,bottom,index,part) for first in groups]
                    actual=packet(reorder(parts,aso) if part else [n for group in parts for n in group]);control=packet([n for first in groups for n in slice_nals(role,first,mode,spatial,mbaff,topology,split,bottom,index,False)])
                    stem=f'{name}-{role}-{index}';files[stem+'.packet']=actual;files[stem+'-control.packet']=control;frames.append((presentation if mbaff else presentation*2+index,role=='seed' and index==0,actual));records.append(dict(packet=stem+'.packet',control=stem+'-control.packet',role=role,presentation=presentation,emits=mbaff or index==1,bottom=bottom,fields=topology))
            files[name+'.mp4']=mux(files[cfgname],frames,32,32,25 if mbaff else 50);files[name+'.yuv']=golden
            cases.append(dict(name=name,mbaff=mbaff,split=split,aso=aso,mode=mode,spatial=spatial,configuration=cfgname,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned two-reference interlaced B A/B/C, physical parity/scan oracle, no private/external content, commands or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
