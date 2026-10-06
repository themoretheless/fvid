#!/usr/bin/env python3
"""Owned CABAC B-direct frame between native non-paired field references."""
import argparse,itertools,json,tempfile,hashlib
from pathlib import Path
from generate_avc_unified_cabac_field_samples import prediction,write_residual
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from generate_avc_field_frame_samples import emit
from avc_fixture_mp4 import mux

def b_frame(address,spatial,init,mode,number=3,poc=8):
    b=Writer();b.ue(address);b.ue(1);b.ue(0);b.u(number,4);b.u(0);b.u(poc,4)
    b.u(int(spatial));b.u(0);b.u(0);b.u(0);b.ue(init);b.se(24);b.ue(1)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(24,int(mode=='skip'))
    if mode!='skip':
        c.decision(27,0)
        if mode.startswith('residual'):write_residual(c,False,'all',int(mode=='residual-negative'))
        else:
            for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-unified-b-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,init,aso,spatial,source,mode in itertools.product([8,10],[False,True],range(3),[False,True],[False,True],['intra','zero','motion'],['skip','direct','residual-positive','residual-negative']):
            order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    poc=number*4+i
                    frames.append((poc,number==0 and i==0,packet([field(bottom,poc,a,'positive' if (a==0)!=bool(number) else 'negative',1,biased=bottom,frame_num=number) for a in addresses])))
            for i,bottom in enumerate(order):
                if source=='intra':ns=[field(bottom,12+i,a,'negative' if a==0 else 'positive',1,biased=bottom,frame_num=2,forget_short=(1,) if i==0 else ()) for a in addresses]
                else:ns=[prediction(a,2,12+i,init,False,bottom=bottom,target=bottom,memory_ops=[(1,(1,))] if i==0 else [],motion=(4 if not bottom else -4,2 if not bottom else -2) if source=='motion' else (0,0)) for a in addresses]
                frames.append((12+i,False,packet(ns)))
            frames.append((8,False,packet([b_frame(a,spatial,init,mode) for a in ([3,1,2,0] if aso else range(4))])))
            for i,bottom in enumerate(order):
                frames.append((14+i,False,packet([prediction(a,3,14+i,init,False,bottom=bottom,target=order[1],target_frame=1,memory_ops=[(1,(1,)),(1,(2,))] if i==0 else []) for a in addresses])))
            frames.append((16,False,packet([prediction(a,4,0,init,False) for a in ([3,1,2,0] if aso else range(4))])))
            name=f'avc-unified-b-{depth}bit-'+('bottom' if reverse else 'top')+f'-init{init}-'+('spatial' if spatial else 'temporal')+f'-{source}-{mode}'+('-aso' if aso else '')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration(depth,max_refs=4),frames,9216*(2 if depth==10 else 1)))
    invalid=[]
    for i,bottom in enumerate([False,True]):
        invalid.append((i,i==0,packet([field(bottom,i,a,'positive' if a==0 else 'negative',1,biased=bottom) for a in [0,1]])))
    for i,bottom in enumerate([False,True]):
        invalid.append((8+i,False,packet([prediction(a,1,8+i,0,False,bottom=bottom,memory_ops=[(1,(1,))] if i==0 else [],motion=(0,0)) for a in [0,1]])))
    invalid.append((4,False,packet([b_frame(a,False,0,'direct',number=2,poc=4) for a in range(4)])))
    invalid_name='avc-unified-b-invalid-temporal-reference.mp4'
    invalid_data=mux(configuration(8),invalid,32,32,50);(root/invalid_name).write_bytes(invalid_data)
    (root/'avc-unified-b-generated.json').write_text(json.dumps(dict(generator='owned CABAC B direct crossing non-paired field reference storage',fixtures=records,refusal=dict(file=invalid_name,sha256=hashlib.sha256(invalid_data).hexdigest())),indent=2)+'\n')
if __name__=='__main__':main()
