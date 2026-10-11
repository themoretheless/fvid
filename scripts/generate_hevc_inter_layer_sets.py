#!/usr/bin/env python3
"""Owned four-layer VPSs, access-unit statuses and a short reference atlas.
Acceptance is for DPB selection/list binding, not coded scalable playback.
"""
import itertools
import json
from pathlib import Path
from generate_hevc_layered_sps import ptl, ue, packed, escape, representation, vps as two_layer_vps
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
IDS=[0,2,5,9]
EDGES=[(1,0),(2,0),(3,0),(3,1),(3,2)]
def vps(views,limits=(7,7,7),one=False,size=(8,8),default=False,sublayers=None):
    b='0000'+'11'+f'{3:06b}'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(3)+ue(1)+ue(0)  # base ordering
    b+=f'{9:06b}'+ue(1)+''.join('1' if i in IDS else '0' for i in range(10))+'0'+'1'
    b+='1'*((-len(b))%8)+ptl(False,level=90)
    b+='0'+''.join('1' if i==1 else '0' for i in range(16))+'001'+'1'
    for index,layer in enumerate(IDS[1:],1):b+=f'{layer:06b}{index:02b}'
    b+=('0000' if all(view==0 for view in views) else '0100'+''.join(f'{view:04b}' for view in views))
    b+=''.join(str(int((i,j) in EDGES)) for i in range(1,4) for j in range(i))
    b+=('0' if sublayers is None else '1'+''.join(f'{tid:03b}' for tid in sublayers))+'1'
    for ref in range(4):
        for dep in range(ref+1,4):
            if (dep,ref) in EDGES:b+=f'{limits[ref] if dep==3 else 7:03b}'
    b+=str(int(default)) # default-reference-layer activation
    b+=ue(1)+ue(0)+'00'+'1111' # PTL count, no extra OLS, all outputs/PTL1
    f=dict(size=list(size),chroma=1,separate=False,depth=[8,8],crop=[0]*4)
    b+=ue(0)+representation(f)+str(int(one))+'0' # unaligned POC
    b+='1' # per-temporal DPB
    for tid in range(3):
        if tid:b+='1'
        b+=''.join(ue(3+tid) for _ in IDS)+ue(1+tid)+ue(0)
    b+=ue(0)+'1'+'10'+ue(0)+'0'+'0'+'1'
    return b'\x40\x01'+escape(packed(b))
def status(layer,au,poc,kind=1,tid=0,discardable=False):
    return dict(layer=layer,au=au,poc=poc,kind=kind,tid=tid,discardable=discardable)
def main():
    configs={
        'views':([4,9,1,6],[7,7,7],False),
        'zero':([0]*4,[7]*3,False),
        'below':([9,1,12,6],[7]*3,False),
        'equal-base':([6,9,1,6],[7]*3,False),
        'temporal':([4,9,1,6],[0,1,2],False),
        'one':([0]*4,[7]*3,True),
    }
    for name,(views,limits,one) in configs.items():(D/f'hevc-inter-layer-set-{name}.vps').write_bytes(vps(views,limits,one))
    independent,_,_=two_layer_vps(dict(size=[8,8],chroma=1,separate=False,depth=[8,8],crop=[0]*4),independent=True)
    (D/'hevc-inter-layer-set-independent.vps').write_bytes(independent)
    pictures=[];raw=b'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C420\n'
    for au in (40,41):
        for i,layer in enumerate(IDS):
            s=status(layer,au,100)
            planes=[[(au*3+i*31+x*7+y*11+c*29)%256 for y in range(8 if c==0 else 4) for x in range(8 if c==0 else 4)] for c in range(3)]
            pictures.append(dict(status=s,planes=planes));raw+=b'FRAME\n'+bytes(sum(planes,[]))
    (D/'hevc-inter-layer-set-atlas.y4m').write_bytes(raw)
    cases=[]
    def add(label,config='views',selected=None,cur=None,changes=None,drop=None,duplicate=None,error=None):
        current=status(9,41,100) if cur is None else cur
        selected=[0,2,5] if selected is None else selected
        sources=[dict(p['status'],frame=i) for i,p in enumerate(pictures)]
        for source in sources:
            if source['au']==current['au']:source['tid']=current['tid']
        for layer,updates in (changes or {}).items():
            for p in sources:
                if p['layer']==layer and p['au']==41:p.update(updates)
        if drop is not None:sources=[p for p in sources if not(p['layer']==drop and p['au']==41)]
        if duplicate is not None:sources.append(next(dict(p) for p in sources if p['layer']==duplicate and p['au']==41))
        groups=[[],[]];frames=[[],[]]
        if error is None:
            views=configs[config][0];base=views[0];view=views[IDS.index(current['layer'])]
            for layer in selected:
                p=next(p for p in sources if p['au']==current['au'] and p['layer']==layer)
                rv=views[IDS.index(layer)]
                # Whether reference is beyond current view relative to base.
                group=0 if (view<=base and view<=rv) or (view>=base and view>=rv) else 1
                groups[group].append([layer,p['poc']]);frames[group].append(p['frame'])
        cases.append(dict(label=label,vps=f'hevc-inter-layer-set-{config}.vps',current=current,selected=selected,sources=sources,groups=groups,frames=frames,error=error))
    for name in ('views','zero','below','equal-base'):
        for mask in range(8):add(f'{name}-mask-{mask}',config=name,selected=[layer for i,layer in enumerate(IDS[:3]) if mask>>i&1])
    # Earlier AU pictures intentionally repeat layer+POC with different pixels.
    # F.8.1.5 requires equal derived POC, independently of coded POC LSB.
    add('derived-poc-mismatch',changes={0:dict(poc=97)},error='POC disagrees')
    add('zero-count-one-flag',config='one',selected=[])
    for layer in IDS[:3]:add(f'one-{layer}',config='one',selected=[layer])
    add('one-violation',config='one',error='order/count')
    for current_type,ref_type in itertools.product((6,7),(8,9)):
        add(f'radl{current_type}-rasl{ref_type}',cur=status(9,41,100,current_type),changes={0:dict(kind=ref_type)},error='RADL')
    for current_type in (0,1,8,9,16,19,21):
        add(f'non-radl-{current_type}',cur=status(9,41,100,current_type),changes={0:dict(kind=8)})
    add('radl-with-radl',cur=status(9,41,100,6),changes={0:dict(kind=6)})
    add('discardable',changes={2:dict(kind=0,discardable=True)},error='discardable picture')
    add('illegal-discardable-r',changes={2:dict(discardable=True)},error='picture status')
    add('missing-selected-layer',drop=2,error='missing from access unit')
    add('ambiguous-selected-layer',duplicate=2,error='ambiguous in access unit')
    add('old-au-is-not-substitute',cur=status(9,42,100),error='missing from access unit')
    add('same-au-same-layer-different-poc',duplicate=2,error='ambiguous in access unit')
    cases[-1]['sources'][-1]['poc']=106
    add('out-of-order',selected=[2,0],error='order/count')
    add('duplicate-selection',selected=[0,0],error='order/count')
    add('self',selected=[9],error='not a direct')
    add('absent-layer',selected=[3],error='absent from VPS')
    add('reserved-current-layer',cur=status(63,41,100),error='picture status')
    add('non-vcl-current',cur=status(9,41,100,32),error='picture status')
    add('reserved-temporal-current',cur=status(9,41,100,1,7),error='picture status')
    add('current-temporal-overflow',cur=status(9,41,100,1,3),error='current picture exceeds')
    add('source-temporal-mismatch',changes={2:dict(tid=1)},error='temporal identity')
    add('source-temporal-overflow',changes={2:dict(tid=3)},error='reference picture exceeds')
    add('source-reserved-type',changes={2:dict(kind=15)},error='picture status')
    add('irap-reference-temporal',changes={2:dict(kind=19,tid=1)},error='picture status')
    for tid in (0,1,2):
        for layer in IDS[:3]:
            limit=configs['temporal'][1][IDS.index(layer)]
            add(f'temporal-{tid}-ref{layer}',config='temporal',selected=[layer],cur=status(9,41,100,1,tid),error=None if limit>tid else 'temporal admission')
    for kind in (16,17,18,19,20,21):
        add(f'zero-limit-irap-{kind}',config='temporal',selected=[0],changes={0:dict(kind=kind)})
    add('zero-limit-irap-nonzero-current',config='temporal',selected=[0],cur=status(9,41,100,1,1),changes={0:dict(kind=19,tid=0)},error='temporal identity')
    # Lower-layer current picture only admits its own direct dependency.
    add('lower-current',cur=status(5,41,100),selected=[0])
    add('non-direct-existing-layer',cur=status(5,41,100),selected=[2],error='not a direct')
    list_cases=[]
    for kind in (16,17,18,19,20,21):
        for list_id in (0,1):
            for mod in ([1,1],None,[0,0]):
                list_cases.append(dict(vps='hevc-inter-layer-set-independent.vps',current=status(3,41,100,kind),selected=[],temporal=[[[3,99]],[],[]] if kind!=21 else [[],[],[[3,99]]],current_picture=True,active=2,list=list_id,modification=mod,expected=None,error='IRAP picture cannot use temporal'))
            list_cases.append(dict(vps='hevc-inter-layer-set-independent.vps',current=status(3,41,100,kind),selected=[],temporal=[[[3,99]],[],[]],current_picture=False,active=2,list=list_id,modification=None,expected=None,error='IRAP picture cannot use temporal'))
            list_cases.append(dict(vps='hevc-inter-layer-set-independent.vps',current=status(3,41,100,kind),selected=[],temporal=[[],[],[]],current_picture=False,active=2,list=list_id,modification=None,expected=None,error='only current'))
            list_cases.append(dict(vps='hevc-inter-layer-set-independent.vps',current=status(3,41,100,kind),selected=[],temporal=[[],[],[]],current_picture=True,active=2,list=list_id,modification=None,expected=[dict(layer=3,poc=100,kind='Current')]*2,error=None))
    list_cases.append(dict(vps='hevc-inter-layer-set-independent.vps',current=status(3,41,100,1),selected=[],temporal=[[[3,99]],[],[]],current_picture=False,active=2,list=0,modification=None,expected=[dict(layer=3,poc=99,kind='ShortTerm')]*2,error=None))
    # A dependent CRA may use inter-layer pictures, but no temporal RPS entry.
    for list_id in (0,1):
        list_cases.append(dict(vps='hevc-inter-layer-set-views.vps',current=status(9,41,100,21),selected=[0,2,5],temporal=[[[9,99]],[],[]],current_picture=False,active=3,list=list_id,modification=None,expected=None,error='IRAP picture cannot use temporal'))
    for kind,list_id in itertools.product((16,17,18),(0,1)):
        list_cases.append(dict(vps='hevc-inter-layer-set-views.vps',current=status(9,41,100,kind),selected=[0,2,5],temporal=[[[9,99]],[],[]],current_picture=False,active=1,list=list_id,modification=[list_id],expected=[dict(layer=9,poc=99,kind='ShortTerm')],error=None))
    for c in list_cases:
        if c['error'] is not None:c['expected']=None
    (D/'hevc-inter-layer-sets.json').write_text(json.dumps(dict(pictures=pictures,cases=cases,list_cases=list_cases,provenance='Original eight-frame Y4M atlas and complete four-layer VPS NALs. DPB/access-unit selection and reference-list binding acceptance only, not coded scalable slice playback. No private source or external codec/FFmpeg/network.'),separators=(',',':'))+'\n')
    print('Inter-layer set cases:',len(cases),'list admission cases:',len(list_cases),flush=True)
if __name__=='__main__':main()
