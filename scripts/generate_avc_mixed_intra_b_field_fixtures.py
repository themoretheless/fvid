#!/usr/bin/env python3
"""Original mixed I/B slices in complementary fields, with own scalar pixels."""
import argparse
import json
from pathlib import Path
from generate_avc_field_pcm_samples import Writer
from generate_avc_switching_field_fixtures import DEST, configuration, pcm, weave, mux
from generate_avc_mixed_intra_sp_field_fixtures import replace_mb, backdrop
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import compensate, verify_jm


def slice_nal(bottom, reverse, address, mode, intra_kind, prediction, source, spatial, skip=False):
    is_intra=intra_kind is not None
    b=Writer();b.ue(address);b.ue(2 if is_intra else 1);b.ue(0)
    b.u(1,4);b.u(1);b.u(int(bottom));b.u(2+int(bottom!=reverse),4)
    if not is_intra:
        b.u(int(spatial)) # slice-local direct prediction mode
        b.u(0);b.u(0);b.u(0) # default active counts, no list reordering
    # Non-reference slices have no decoded-reference marking syntax.
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if skip and not is_intra:
        b.ue(1);return b.nal(0x01)
    if is_intra:
        if intra_kind=='pcm':
            b.ue(25);b.align()
            for plane,width,size in zip(source,[32,16,16],[16,8,8]):
                for row in range(size):
                    for col in range(size):b.u(plane[row*width+address*size+col],8)
        elif intra_kind=='i4':
            b.ue(0)
            for _ in range(16):b.u(1)
            b.ue(0);b.ue(3)
        else:b.ue(3);b.ue(0);b.se(0);b.u(1)
    else:
        b.ue(0);b.ue(prediction) # no skip, B_L0/L1/Bi_16x16
        for _ in range(0 if prediction==0 else (2 if prediction==3 else 1)):b.se(0);b.se(0)
        b.ue(0)
    return b.nal(0x01)


def follow_p(bottom,reverse):
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom))
    b.u(4+int(bottom!=reverse),4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(2)
    return b.nal(0x41)


def observable_weighting_controls(cases):
    keys=['intra_kind','prediction','spatial','reverse','b_mb','aso','mode']
    indexed={tuple(c[k] for k in keys)+(c['bipred'],):c for c in cases}
    count=0
    for case in cases:
        if case['bipred']!=0:continue
        other=indexed[tuple(case[k] for k in keys)+(2,)]
        count+=(DEST/case['reference']).read_bytes()!=(DEST/other['reference']).read_bytes()
    assert count>0,'implicit weighting controls must change pixels'
    return count


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional explicit all-plane cross-check only')
    args=parser.parse_args()
    cases=[]
    for bipred in [0,2]:
        config=configuration(8,max_refs=3,bipred=bipred)
        for intra_kind in ['pcm','i4','i16']:
            for prediction in [0,1,2,3]:
                for spatial in ([False,True] if prediction==0 else [True]):
                    for reverse in [False,True]:
                        for b_mb in [0,1]:
                            for aso in [False,True]:
                                for mode in [0,1,2]:
                                    fields={b:backdrop(b,intra_kind) for b in [False,True]};initial={b:[p[:] for p in ps] for b,ps in fields.items()}
                                    frames=[];packets=[];order=[True,False] if reverse else [False,True]
                                    def append(nals,idr=False):
                                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                        frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                    raw=bytearray(weave(fields))
                                    for bottom in order:
                                        source=[[v+7 for v in p] for p in smooth(bottom)] if intra_kind=='pcm' else [[128]*512,[128]*128,[128]*128]
                                        nals=[slice_nal(bottom,reverse,mb,mode,None if mb==b_mb else intra_kind,prediction,source,spatial) for mb in range(2)]
                                        append(nals[::-1] if aso else nals)
                                        # Identical default B lists swap the first two L1
                                        # fields: L0[0] is same parity, L1[0] opposite.
                                        l0=initial[bottom]
                                        l1=[initial[not bottom][0]]+[compensate(p,16,8,(0,2 if bottom else -2),True) for p in initial[not bottom][1:]]
                                        fields[bottom]=[p[:] for p in l0] if prediction==1 else (
                                            [p[:] for p in l1] if prediction==2 else
                                            [[max(0,min(255,(-64*a+128*b+32)//64)) if bipred==2 and bottom==reverse else (a+b+1)//2 for a,b in zip(p,q)] for p,q in zip(l0,l1)])
                                        replace_mb(fields[bottom],source,1-b_mb)
                                        if intra_kind!='pcm':
                                            fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},switching_blocks={1-b_mb})
                                                for c,(p,w,h) in enumerate(zip(fields[bottom],[32,16,16],[16,8,8]))]
                                    raw.extend(weave(fields))
                                    # B/I picture is non-reference: P must still use the initial PCM pair.
                                    for bottom in order:append([follow_p(bottom,reverse)])
                                    raw.extend(weave(initial))
                                    name=f'avc-mixed-intra-b-field-{intra_kind}-pred{prediction}-'+('bottom-first' if reverse else 'top-first')+f'-b{b_mb}-mode{mode}-weight{bipred}-spatial{int(spatial)}'+('-aso' if aso else '')
                                    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                    (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                    cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,intra_kind=intra_kind,prediction=prediction,reverse=reverse,b_mb=b_mb,aso=aso,mode=mode,bipred=bipred,spatial=spatial))
    (DEST/'avc-mixed-intra-b-fields.json').write_text(json.dumps(dict(cases=cases,provenance='Original PCM fields, mixed non-reference I/B fields, subsequent P fields. Own zero-motion prediction and scalar deblocking. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed I/B field streams:',len(cases), 'observable implicit weighting controls:',observable_weighting_controls(cases))
    if args.jm_decoder:verify_jm(cases,args.jm_decoder,all_planes=True)


if __name__=='__main__':main()
