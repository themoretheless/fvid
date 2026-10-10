#!/usr/bin/env python3
"""Original mixed CABAC I/B complementary fields with own scalar pixels."""
import argparse
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from generate_avc_field_cabac_samples import configuration
from generate_avc_field_b_cabac_mixed_samples import mb_type
from generate_avc_switching_field_fixtures import DEST, weave, mux
from generate_avc_mixed_intra_sp_field_fixtures import replace_mb, backdrop
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import compensate, verify_jm


def aligned_header(bottom,reverse,address,kind,mode,init,frame=1,reference=False,idr=False,poc=None,spatial=True):
    b=Writer();b.ue(address);b.ue(kind);b.ue(0);b.u(frame,4);b.u(1);b.u(int(bottom))
    if idr:b.ue(0)
    b.u((2*frame if poc is None else poc)+int(bottom!=reverse),4)
    if kind==1:b.u(int(spatial))
    if kind!=2:
        b.u(0);b.u(0)
        if kind==1:b.u(0)
    if reference:
        b.u(0)
        if idr:b.u(0)
    if kind!=2:b.ue(init)
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    while len(b.bits)%8:b.u(1)
    return b


def pcm_body(b,c,address,source):
    c.decision(3,1);b.bits.extend(c.finish())
    while len(b.bits)%8:b.u(0)
    for plane,width,size in zip(source,[32,16,16],[16,8,8]):
        for row in range(size):
            for col in range(size):b.u(plane[row*width+address*size+col],8)
    c.low=0;c.range=510;c.width=9
    b.bits.extend(c.finish())


def initial_pcm(bottom,reverse,address,source,idr):
    b=aligned_header(bottom,reverse,address,2,1,0,frame=0,reference=True,idr=idr)
    pcm_body(b,CabacWriter(-1,26),address,source)
    return b.nal(0x65 if idr else 0x41,trailing=False)


def slice_nal(bottom,reverse,address,mode,intra_kind,prediction,source,spatial,init,skip=False):
    kind=2 if intra_kind is not None else (0 if prediction<0 else 1)
    b=aligned_header(bottom,reverse,address,kind,mode,init,spatial=spatial)
    c=CabacWriter(-1 if kind==2 else init,26)
    if intra_kind=='pcm':
        pcm_body(b,c,address,source)
        return b.nal(0x01,trailing=False)
    if intra_kind=='i4':
        c.decision(3,0)
        for _ in range(16):c.decision(68,1)
        c.decision(64,0)
    elif intra_kind=='i16':
        c.decision(3,1);c.range-=2;c.renormalize()
        c.decision(6,0);c.decision(7,0);c.decision(9,1);c.decision(10,0)
        c.decision(64,0);c.decision(60,0);c.decision(88,0)
    else:
        if kind==0:
            c.decision(11,int(prediction==-1))
            if prediction==-1:
                b.bits.extend(c.finish());return b.nal(0x01,trailing=False)
            c.decision(14,0);c.decision(15,0);c.decision(16,0)
        else:
            c.decision(24,int(skip))
            if skip:
                b.bits.extend(c.finish());return b.nal(0x01,trailing=False)
            mb_type(c,prediction,0)
        for _ in range(0 if prediction==0 else (2 if prediction==3 else 1)):
            c.mvd(0,0,0);c.mvd(1,0,0)
    if intra_kind!='i16':
        for ctx in [73,74,75,76,77]:c.decision(ctx,0)
    b.bits.extend(c.finish());return b.nal(0x01,trailing=False)


def follow_p(bottom,reverse):
    b=aligned_header(bottom,reverse,0,0,1,0,frame=1,reference=True,poc=4)
    c=CabacWriter(0,26);c.decision(11,1);c.range-=2;c.renormalize();c.decision(11,1)
    b.bits.extend(c.finish());return b.nal(0x41,trailing=False)


def observable_weighting_controls(cases):
    keys=['intra_kind','prediction','spatial','reverse','b_mb','aso','mode','init']
    indexed={tuple(c[k] for k in keys)+(c['bipred'],):c for c in cases}
    count=0
    for case in cases:
        if case['bipred']!=0:continue
        other=indexed[tuple(case[k] for k in keys)+(2,)]
        count+=(DEST/case['reference']).read_bytes()!=(DEST/other['reference']).read_bytes()
    assert count>0,'implicit weighting controls must change pixels'
    return count


def observable_filter_controls(cases):
    keys=['intra_kind','prediction','spatial','reverse','b_mb','aso','bipred','init']
    indexed={tuple(c[k] for k in keys)+(c['mode'],):c for c in cases}
    counts={'i4':0,'i16':0}
    for case in cases:
        if case['mode']!=0 or case['intra_kind']=='pcm':continue
        other=indexed[tuple(case[k] for k in keys)+(2,)]
        counts[case['intra_kind']]+=(DEST/case['reference']).read_bytes()!=(DEST/other['reference']).read_bytes()
    assert all(counts.values()),counts
    return counts


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional explicit all-plane cross-check only')
    args=parser.parse_args()
    cases=[]
    for init in range(3):
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
                                        for i,bottom in enumerate(order):append([initial_pcm(bottom,reverse,a,fields[bottom],i==0) for a in [0,1]],i==0)
                                        raw=bytearray(weave(fields))
                                        for bottom in order:
                                            source=[[v+7 for v in p] for p in smooth(bottom)] if intra_kind=='pcm' else [[128]*512,[128]*128,[128]*128]
                                            nals=[slice_nal(bottom,reverse,mb,mode,None if mb==b_mb else intra_kind,prediction,source,spatial,init) for mb in range(2)]
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
                                        name=f'avc-mixed-cabac-intra-b-field-{intra_kind}-pred{prediction}-'+('bottom-first' if reverse else 'top-first')+f'-b{b_mb}-mode{mode}-weight{bipred}-spatial{int(spatial)}-init{init}'+('-aso' if aso else '')
                                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                        (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,intra_kind=intra_kind,prediction=prediction,reverse=reverse,b_mb=b_mb,aso=aso,mode=mode,bipred=bipred,spatial=spatial,init=init))
    (DEST/'avc-mixed-cabac-intra-b-fields.json').write_text(json.dumps(dict(cases=cases,provenance='Original PCM fields, mixed non-reference CABAC I/B fields, subsequent P fields. Own zero-motion prediction and scalar deblocking. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed CABAC I/B field streams:',len(cases), 'observable implicit weighting controls:',observable_weighting_controls(cases))
    print('Observable external filtering controls:',observable_filter_controls(cases))
    if args.jm_decoder:verify_jm(cases,args.jm_decoder,all_planes=True)


if __name__=='__main__':main()
