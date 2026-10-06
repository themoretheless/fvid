#!/usr/bin/env python3
"""Owned CABAC retained non-paired fields across full-frame prediction."""
import argparse,itertools,json,tempfile
from pathlib import Path
from generate_avc_field_cabac_samples import configuration,field
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from generate_avc_field_frame_samples import emit

def prediction(address,number,poc,init,skip,bottom=None,target=None,clear=False,residual=None,target_frame=0):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(number,4);b.u(int(bottom is not None))
    if bottom is not None:b.u(int(bottom))
    b.u(poc,4);b.u(0)
    b.u(int(target is not None))
    if target is not None:b.ue(0);b.ue(5-2*target_frame+int(bottom!=target));b.ue(3)
    ops=[(1,(0,))] if number==2 else ([(1,(1,)),(1,(2,))] if clear else [])
    # Even an empty adaptive list prevents sliding while appending field two.
    adaptive=bool(ops) or bottom is not None
    b.u(int(adaptive))
    if adaptive:
        for opcode,values in ops:
            b.ue(opcode)
            for value in values:b.ue(value)
        b.ue(0)
    b.ue(init);b.se(24);b.ue(1)
    while len(b.bits)%8:b.u(1)
    c=CabacWriter(init,50);c.decision(11,int(skip))
    if not skip:
        c.decision(14,0);c.decision(15,0);c.decision(16,0)
        for component in range(2):c.mvd(component,0 if bottom is not None else 4,0)
        if residual is None:
            for ctx in [73,74,75,76,77]:c.decision(ctx,0)
        else:write_residual(c,bottom is not None,*residual)
    b.bits.extend(c.finish());return b.nal(0x41,trailing=False)

def write_residual(c,is_field,mode,negative):
    luma=mode in ['empty','luma','all'];chroma=mode in ['empty','chroma','all']
    if luma:
        for _ in range(4):c.decision(73,1)
    else:
        for ctx in [73,74,75,76]:c.decision(ctx,0)
    c.decision(77,int(chroma))
    if chroma:c.decision(81,1)
    c.decision(60,0)
    if luma:
        done=set()
        for block in range(16):
            bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
            inc=int((bx-1,by) in done)+2*int((bx,by-1) in done)
            c.decision(93+inc,int(mode!='empty'))
            if mode!='empty':
                base=306 if is_field else 134;last=367 if is_field else 195
                c.decision(base,0);c.decision(base+1,1);c.decision(last+1,1);c.decision(248,0);c.bypass((block%2)^negative);done.add((bx,by))
    if chroma:
        for component in range(2):
            c.decision(97,int(mode!='empty'))
            if mode!='empty':c.decision(321 if is_field else 149,1);c.decision(382 if is_field else 210,1);c.decision(258,0);c.bypass(component^negative)
        for component in range(2):
            done=set()
            for block in range(4):
                bx=block%2;by=block//2;inc=int((bx-1,by) in done)+2*int((bx,by-1) in done)
                c.decision(101+inc,int(mode!='empty'))
                if mode!='empty':c.decision(324 if is_field else 152,1);c.decision(385 if is_field else 213,1);c.decision(267,0);c.bypass(((block+component)%2)^negative);done.add((bx,by))

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path,required=True);args=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    packet=lambda ns:b''.join(len(n).to_bytes(4,'big')+n for n in ns)
    with tempfile.TemporaryDirectory(prefix='fvid-unified-cabac-') as tmp:
        d=Path(tmp);cfg=d/'decoder.cfg';cfg.write_text('')
        for depth,reverse,init,frame_skip,field_skip,aso in itertools.product([8,10],[False,True],range(3),[False,True],[False,True],[False,True]):
            order=[True,False] if reverse else [False,True];addresses=[1,0] if aso else [0,1];frames=[]
            for number in [0,1]:
                for i,bottom in enumerate(order):
                    poc=(0 if number==0 else 8)+i
                    ns=[field(bottom,poc,a,'positive' if (a==0) != bool(number) else 'negative',1,biased=bottom,frame_num=number,forget_short=(1,) if number==1 and i==0 else ()) for a in addresses]
                    frames.append((poc,number==0 and i==0,packet(ns)))
            frames.append((12,False,packet([prediction(a,2,12,init,frame_skip) for a in ([3,1,2,0] if aso else range(4))])))
            for i,bottom in enumerate(order):
                frames.append((14+i,False,packet([prediction(a,3,14+i,init,field_skip,bottom=bottom,target=order[1],clear=i==0) for a in addresses])))
            frames.append((16,False,packet([prediction(a,4,0,init,frame_skip) for a in ([3,1,2,0] if aso else range(4))])))
            name=f'avc-unified-cabac-{depth}bit-'+('bottom' if reverse else 'top')+f'-init{init}-frame'+('skip' if frame_skip else 'motion')+'-field'+('skip' if field_skip else 'coded')+('-aso' if aso else '')
            records.append(emit(root,d,cfg,args.jm_decoder,name,configuration(depth),frames,7680*(2 if depth==10 else 1)))
    (root/'avc-unified-cabac-generated.json').write_text(json.dumps(dict(generator='owned CABAC retained non-paired fields and frame prediction',fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
