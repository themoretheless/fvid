#!/usr/bin/env python3
"""Owned explicit and weighted CABAC B through canonical field/frame views."""
import argparse,itertools,json,tempfile
from pathlib import Path
from generate_avc_unified_cabac_field_samples import prediction
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from generate_avc_paff_cabac_weight_samples import tree
from generate_avc_field_frame_samples import emit

def explicit(address,init,weight,selected,shape):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(3,4);b.u(0);b.u(8,4);b.u(0)
    b.u(1);b.ue(1);b.ue(1);b.u(0);b.u(0)
    if weight in ['identity','weighted']:
        b.ue(2);b.ue(1)
        for table in [[(3,5,[(1,-7),(3,8)]),(-2,90,[(0,24),(-1,48)])],[(5,-24,[(3,7),(1,-8)]),(1,9,[(2,-10),(4,3)])]]:
            for w,o,chroma in table:
                b.u(int(weight=='weighted'))
                if weight=='weighted':b.se(w);b.se(o)
                b.u(int(weight=='weighted'))
                if weight=='weighted':
                    for cw,co in chroma:b.se(cw);b.se(co)
    b.ue(init);b.se(24);b.ue(1)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(24,0)
    tree(c,{'l0':'100','l1':'101','bi16':'110000','bi16x8':'1111000','bi8x16':'1111001','bi8x8':'111111'}[shape],27)
    if shape=='bi8x8':
        for _ in range(4):tree(c,'11000',36)
    origins={'l0':[(0,0)],'l1':[(0,0)],'bi16':[(0,0)],'bi16x8':[(0,0),(0,2)],'bi8x16':[(0,0),(2,0)],'bi8x8':[(0,0),(2,0),(0,2),(2,2)]}[shape]
    width,height=(4,2) if shape=='bi16x8' else (2,4) if shape=='bi8x16' else (2,2) if shape=='bi8x8' else (4,4)
    lists=[0] if shape=='l0' else [1] if shape=='l1' else [0,1]
    for list in lists:
        references=[None]*16
        for x,y in origins:
            left=references[y*4+x-1] if x else None;top=references[(y-1)*4+x] if y else None
            inc=int(left is not None and left>0)+2*int(top is not None and top>0)
            c.decision(54+inc,selected)
            if selected:c.decision(58,0)
            for dy in range(height):
                for dx in range(width):references[(y+dy)*4+x+dx]=selected
    for list in lists:
        magnitudes=[[0,0] for _ in range(16)]
        for x,y in origins:
            vector=[-4 if list else 4,0]
            for component,value in enumerate(vector):
                neighbour=(magnitudes[y*4+x-1][component] if x else 0)+(magnitudes[(y-1)*4+x][component] if y else 0)
                c.mvd(component,value,neighbour)
            for dy in range(height):
                for dx in range(width):magnitudes[(y+dy)*4+x+dx]=[abs(v) for v in vector]
    for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-unified-explicit-b-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,init,aso,source,weight,selected,shape in itertools.product([8,10],[False,True],range(3),[False,True],['intra','motion'],['average','identity','weighted','implicit'],[0,1],['l0','l1','bi16','bi16x8','bi8x16','bi8x8']):
            order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    poc=number*4+i
                    frames.append((poc,number==0 and i==0,packet([field(bottom,poc,a,'positive' if (a==0)!=bool(number) else 'negative',1,biased=bottom,frame_num=number) for a in addresses])))
            for i,bottom in enumerate(order):
                if source=='intra':ns=[field(bottom,12+i,a,'negative' if a==0 else 'positive',1,biased=bottom,frame_num=2,forget_short=(1,) if i==0 else ()) for a in addresses]
                else:ns=[prediction(a,2,12+i,init,False,bottom=bottom,target=bottom,memory_ops=[(1,(1,))] if i==0 else [],motion=(4 if not bottom else -4,2 if not bottom else -2)) for a in addresses]
                frames.append((12+i,False,packet(ns)))
            frames.append((8,False,packet([explicit(a,init,weight,selected,shape) for a in ([3,1,2,0] if aso else range(4))])))
            for i,bottom in enumerate(order):
                frames.append((14+i,False,packet([prediction(a,3,14+i,init,False,bottom=bottom,target=order[1],target_frame=1,memory_ops=[(1,(1,)),(1,(2,))] if i==0 else []) for a in addresses])))
            frames.append((16,False,packet([prediction(a,4,0,init,False) for a in ([3,1,2,0] if aso else range(4))])))
            name=f'avc-unified-explicit-b-{depth}bit-'+('bottom' if reverse else 'top')+f'-init{init}-{source}-{weight}-ref{selected}-{shape}'+('-aso' if aso else '')
            config=configuration(depth,max_refs=4,bipred=2 if weight=='implicit' else 1 if weight in ['identity','weighted'] else 0)
            records.append(emit(root,d,cfg,args.jm_decoder,name,config,frames,9216*(2 if depth==10 else 1)))
    (root/'avc-unified-explicit-b-generated.json').write_text(json.dumps(dict(generator='owned CABAC explicit weighted B through canonical non-paired fields',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
