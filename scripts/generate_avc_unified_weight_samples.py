#!/usr/bin/env python3
"""Owned weighted P references through canonical field/frame transitions."""
import argparse,itertools,json,tempfile
from pathlib import Path
from generate_avc_unified_cabac_field_samples import prediction
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_field_frame_samples import emit

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-unified-weight-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,init,aso,stage,source_frame,weights,mode in itertools.product([8,10],[False,True],range(3),[False,True],['frame','field','both'],[False,True],['identity','balanced','negative'],['skip','coded','residual']):
            order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    poc=(0 if number==0 else 8)+i
                    ns=[field(bottom,poc,a,'positive' if (a==0) != bool(number) else 'negative',1,biased=bottom,frame_num=number,forget_short=(1,) if number==1 and i==0 else ()) for a in addresses]
                    frames.append((poc,number==0 and i==0,packet(ns)))
            residual=('all',1) if mode=='residual' else None;skip=mode=='skip'
            frames.append((12,False,packet([prediction(a,2,12,init,skip,residual=residual,weights=weights if stage!='field' else 'identity') for a in ([3,1,2,0] if aso else range(4))])))
            for i,bottom in enumerate(order):
                frames.append((14+i,False,packet([prediction(a,3,14+i,init,skip,bottom=bottom,target=order[1],clear=i==0 and not source_frame,target_frame=2 if source_frame else 0,residual=residual,weights=weights if stage!='frame' else 'identity') for a in addresses])))
            frames.append((16,False,packet([prediction(a,4,0,init,skip,weights='identity') for a in ([3,1,2,0] if aso else range(4))])))
            name=f'avc-unified-weight-{depth}bit-'+('bottom' if reverse else 'top')+f'-init{init}-{stage}-{weights}-{mode}'+('-fromframe' if source_frame else '')+('-aso' if aso else '')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration(depth,weighted=True),frames,7680*(2 if depth==10 else 1)))
    (root/'avc-unified-weight-generated.json').write_text(json.dumps(dict(generator='owned CABAC weighted canonical field/frame references',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
