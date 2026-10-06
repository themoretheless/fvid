#!/usr/bin/env python3
"""Owned retained individual fields referenced after intervening full frames."""
import argparse,itertools,json,tempfile
from pathlib import Path
from generate_avc_field_pcm_samples import Writer,configuration
from generate_avc_field_reference_samples import intra
from generate_avc_field_frame_samples import p_field,full_frame,emit

def retained_field(bottom,target,mode,address,poc,skip,clear):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(3,4);b.u(1);b.u(int(bottom));b.u(poc,4);b.u(0)
    b.u(1)
    if mode=='partial':b.ue(0);b.ue(5+int(bottom!=target))
    else:b.ue(2);b.ue(2+int(bottom==target))
    b.ue(3);b.u(1)
    if clear:
        # Remove both fields of frame2, plus the short field of mixed frame1.
        for delta in ([1,2,4] if mode=='mixed' else [1,2]):b.ue(1);b.ue(delta)
    b.ue(0);b.se(0);b.ue(1);b.ue(1 if skip else 0)
    if not skip:b.ue(0);b.se(0);b.se(0);b.ue(0)
    return b.nal(0x41)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-unified-fields-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,mode,motion,skip,aso in itertools.product([8,10],[False,True],['partial','mixed'],[False,True],[False,True],[False,True]):
            order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
            for i,bottom in enumerate(order):frames.append((i,i==0,packet([intra(bottom,depth,0,i==0,i)])))
            for i,bottom in enumerate(order):frames.append((8+i,False,packet([p_field(bottom,8+i,a,motion,partial=mode=='partial' and i==0,mixed=mode=='mixed' and i==0) for a in addresses])))
            # Free the unwanted complete pair while preserving the individual target.
            operations=((1,(0 if mode=='partial' else 1,)),)
            frames.append((12,False,packet(full_frame(True,'p',False,motion,number=2,poc=12,memory_operations=operations))))
            target=order[1] if mode=='partial' else order[0]
            for i,bottom in enumerate(order):frames.append((14+i,False,packet([retained_field(bottom,target,mode,a,14+i,skip,clear=i==0) for a in addresses])))
            frames.append((16,False,packet(full_frame(True,'p',False,motion,number=4,poc=0))))
            name=f'avc-unified-field-{mode}-{depth}bit-'+('bottom' if reverse else 'top')+('-motion' if motion else '-zero')+('-skip' if skip else '-coded')+('-aso' if aso else '')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration(depth),frames,7680*(2 if depth==10 else 1)))
    (root/'avc-unified-field-generated.json').write_text(json.dumps(dict(generator='owned individual reference fields retained across full frame marking',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
