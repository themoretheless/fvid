#!/usr/bin/env python3
"""Owned paired-layer parameters, two-frame Y4M and composed sample oracles."""
import json,struct
from pathlib import Path
import generate_hevc_layered_sps as g
import generate_hevc_layered_pps as p
from generate_hevc_colour_mapping_pictures import map_planes
from generate_hevc_inter_layer_resampling import derive,planes,oracle
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'

def main():
    cases=[]
    def add(label,rf=1,cf=1,cm=None,**options):
        c=dict(prefix='hevc-reference-samples-'+label,reference_size=[8,8],current_size=[16,16],reference_format=rf,current_format=cf,reference_depth=[8,8],current_depth=[8,8],scaled=[0]*4,region=[0]*4,phases=None,dependency=2,error=None)
        c.update(options)
        if rf != cf:
            # A doubled luma extent can leave a chroma extent unchanged when
            # subsampling changes; equal component regions require phase zero.
            c['phases']=[0,0,8,8]
        if cm is not None:
            c['reference_depth']=list(cm.get('inp',(8,8)))
            c['current_depth']=list(cm.get('out',(8,8)))
        mapping,table=p.colour(**cm) if cm is not None else (None,None)
        base_fmt=dict(g.BASE,size=c['reference_size'],chroma=rf,depth=c['reference_depth'])
        target=dict(g.fmt(cf,depth=c['current_depth'],size=c['current_size']),crop=[0]*4)
        vps,_,_=g.vps(target,base_format=base_fmt,dependency_type=c['dependency'])
        base=g.sps(multilayer=False,independent=True,scaling='explicit',buffers=4,latency=1,coded_depth=c['reference_depth'],coded_size=c['reference_size'],coded_chroma=rf)
        current=g.sps(scaling='default')
        base_pps=p.pps(layer=0)
        locations=[] if c['scaled']==[0]*4 and c['region']==[0]*4 and c['phases'] is None else [dict(id=0,scaled=c['scaled'],region=c['region'],**({'phases':c['phases']} if c['phases'] is not None else {}))]
        pps=p.pps(mapping=mapping,locations=locations)
        for suffix,raw in [('vps',vps),('base.sps',base),('current.sps',current),('base.pps',base_pps),('pps',pps),('mp4',p.movie(vps,base,current,base_pps,pps))]:
            (D/(c['prefix']+'.'+suffix)).write_bytes(raw)
        c['table']=table
        c['frames']=[]
        scale=dict(c,reference_depth=list(table['output']) if table else c['reference_depth'])
        coords=derive(scale)
        depth=max(c['reference_depth']); colour={1:'420',2:'422',3:'444'}[rf]+(f'p{depth}' if depth!=8 else '')
        raw=f'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C{colour}\n'.encode()
        for f in (0,1):
            source=planes(c,f)
            mapped=map_planes(table,rf,8,8,source) if table else None
            output=oracle(scale,mapped if table else source,coords)
            differences=None
            if table and c['current_depth']==c['reference_depth']:
                reverse=map_planes(table,cf,*c['current_size'],oracle(c,source,derive(c)))
                differences=sum(a!=b for pa,pb in zip(output,reverse) for a,b in zip(pa,pb))
            c['frames'].append(dict(input=source,mapped=mapped,output=output,reverse_order_diff_samples=differences))
            flat=sum(source,[]);raw+=b'FRAME\n'+(bytes(flat) if depth==8 else struct.pack('<'+'H'*len(flat),*flat))
        (D/(c['prefix']+'.y4m')).write_bytes(raw)
        cases.append(c)
    patterns=[('bias',dict(pattern='bias')),('swap',dict(pattern='swap')),('partition',dict(pattern='partition',depth=1,part=2)),('adaptive',dict(pattern='partition',depth=1,threshold=(-128,127)))]
    for rf in (1,2,3):
        for cf in (1,2,3):
            add(f'plain-{rf}-{cf}',rf,cf)
            for label,cm in patterns:add(f'{label}-{rf}-{cf}',rf,cf,cm)
    for fmt in (1,2,3):
        add(f'mapped-identity16-{fmt}',fmt,fmt,dict(pattern='bias',out=(16,16),quant=3,delta=3),current_size=[8,8])
        add(f'mixed-output-{fmt}',fmt,fmt,dict(pattern='mixed',out=(10,8)))
        add(f'mixed-input-{fmt}',fmt,fmt,dict(pattern='bias',inp=(10,8),out=(12,10)))
        add(f'offset-{fmt}',fmt,fmt,dict(pattern='partition',depth=1,part=2),current_size=[24,16],scaled=[1,-1,2,1],region=[-1,1,1,-1])
        add(f'identity16-{fmt}',fmt,fmt,reference_depth=[16,16],current_depth=[16,16],current_size=[8,8])
    add('sample-only',cm=dict(pattern='swap'),dependency=0)
    add('motion-only',cm=dict(pattern='swap'),dependency=1,error='disables inter-layer sample prediction')
    add('no-prediction',dependency=3,error='disables inter-layer sample prediction')
    (D/'hevc-reference-samples.json').write_text(json.dumps(dict(cases=cases,provenance='Owned VPS/SPS/PPS + synthetic MP4 configuration reproducers and original two-frame Y4M reference patterns. Independent composed rational colour/spatial oracles. Mixed-depth Y4M is a storage envelope with operation depths explicit. MP4 seed pictures are retained unchanged; configuration/sample operations only, not scalable decoding. No private data, external codec, FFmpeg or network.'),separators=(',',':'))+'\n')
    print('Reference sample cases:',len(cases),flush=True)
if __name__=='__main__':main()
