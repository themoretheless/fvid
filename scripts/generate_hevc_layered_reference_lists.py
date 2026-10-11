#!/usr/bin/env python3
"""Owned reference-picture atlas and independent F.8.3.4 list oracles."""
import itertools,json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BEFORE=[[7,99],[7,98],[7,97]]
AFTER=[[7,101],[7,102],[7,103]]
LONG=[[7,20],[7,10]]
INTER0=[[0,100],[2,100],[3,100]]
INTER1=[[1,100],[4,100]]
CURRENT=[7,100]
GROUPS=[BEFORE,AFTER,LONG,INTER0,INTER1]
def oracle(groups,current,active,modification,list_id):
    # Named normative groups, not decoder source-index selection.
    before,after,long,inter0,inter1=groups
    wrap=lambda values,kind:[dict(picture=p,kind=kind) for p in values]
    temporal_before=wrap(before,'ShortTerm'); temporal_after=wrap(after,'ShortTerm')
    inter_before=wrap(inter0,'InterLayer'); inter_after=wrap(inter1,'InterLayer')
    cycle=(temporal_before+inter_before+temporal_after+wrap(long,'LongTerm')+inter_after if list_id==0 else temporal_after+inter_after+temporal_before+wrap(long,'LongTerm')+inter_before)
    if current:cycle.append(dict(picture=CURRENT,kind='Current'))
    if not cycle and active:return None,'active reference list is empty'
    if modification is not None: output=[cycle[index] for index in modification]
    else:output=(cycle*((active+len(cycle)-1)//len(cycle)))[:active] if cycle else []
    if current and list_id==0 and modification is None and len(cycle)>active and active:output[-1]=dict(picture=CURRENT,kind='Current')
    return output,None

def main():
    pictures=[];raw=b'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C420\n'
    for i,p in enumerate(BEFORE+AFTER+LONG+INTER0+INTER1+[CURRENT]):
        planes=[[(i*17+x*7+y*11+c*29)%256 for y in range(8 if c==0 else 4) for x in range(8 if c==0 else 4)] for c in range(3)]
        raw+=b'FRAME\n'+bytes(sum(planes,[]));pictures.append(dict(id=p,planes=planes))
    (D/'hevc-layered-reference-atlas.y4m').write_bytes(raw)
    cases=[]
    for counts in itertools.product((0,1,3),(0,1,3),(0,1,2),(0,1,3),(0,1,2)):
        groups=[g[:n] for g,n in zip(GROUPS,counts)]
        for current,list_id,active in itertools.product((False,True),(0,1),(0,1,2,7,15)):
            output,error=oracle(groups,current,active,None,list_id)
            cases.append(dict(counts=counts,current=current,list=list_id,active=active,modification=None,expected=output,error=error))
        for current,list_id in itertools.product((False,True),(0,1)):
            n=sum(counts)+current
            if not n:continue
            modification=[n-1,0,n-1]
            output,error=oracle(groups,current,3,modification,list_id)
            cases.append(dict(counts=counts,current=current,list=list_id,active=3,modification=modification,expected=output,error=error))
    def bad(label,error,**options):
        c=dict(label=label,counts=[1,1,1,1,1],current=True,list=0,active=2,modification=None,expected=None,error=error);c.update(options);cases.append(c)
    bad('before-layer','wrong layer or POC',overrides={'0':[[0,99]]})
    bad('before-side','wrong side',overrides={'0':[[7,101]]})
    bad('after-side','wrong side',overrides={'1':[[7,99]]})
    bad('before-order','order is invalid',overrides={'0':[[7,98],[7,99]]})
    bad('after-order','order is invalid',overrides={'1':[[7,102],[7,101]]})
    bad('temporal-repeat','repeat a picture identity',overrides={'2':[[7,99]]})
    bad('layer-repeat','repeat a picture identity',overrides={'4':[[0,100]]})
    bad('layer-self','wrong layer or POC',overrides={'3':[[7,100]]})
    bad('layer-poc','wrong layer or POC',overrides={'3':[[0,99]]})
    bad('reserved-layer','wrong layer or POC',overrides={'3':[[63,100]]})
    bad('current-layer','parameters',current_layer=63)
    bad('list-number','parameters',list=2)
    bad('active-number','parameters',active=16)
    bad('modification-length','parameters',modification=[0])
    bad('modification-index','index out of range',modification=[6,0])
    bad('count','parameters',overrides={'0':[[7,99]]*257})
    (D/'hevc-layered-reference-lists.json').write_text(json.dumps(dict(pictures=pictures,cases=cases,provenance='Original fourteen-frame Y4M atlas with distinct layer/POC images and independently authored F.8.3.4 reference-group/list oracle. List construction and exact-identity resolution acceptance only; no scalable slice/picture playback claim. No private data, external codec, FFmpeg or network.'),separators=(',',':'))+'\n')
    print('Layered reference list cases:',len(cases),flush=True)
if __name__=='__main__':main()
