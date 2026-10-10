#!/usr/bin/env python3
"""Original mixed P/B complementary fields with own prediction/filter oracle."""
import argparse
import json
from pathlib import Path
from generate_avc_switching_field_fixtures import DEST, configuration, pcm, weave, mux
from generate_avc_field_cabac_samples import configuration as cabac_configuration
from generate_avc_field_pcm_samples import Writer
from generate_avc_mixed_intra_b_field_fixtures import slice_nal as cavlc_b, follow_p as cavlc_follow
from generate_avc_mixed_cabac_intra_b_field_fixtures import initial_pcm, slice_nal as cabac_inter, follow_p as cabac_follow
from generate_avc_mixed_intra_sp_field_fixtures import replace_mb
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import compensate, verify_jm


def cavlc_p(bottom,reverse,address,mode,skip):
    b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom))
    b.u(2+int(bottom!=reverse),4);b.u(0);b.u(0);b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if skip:b.ue(1)
    else:b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(0)
    return b.nal(0x01)


def observable_controls(cases):
    keys=['entropy','skip','prediction','spatial','bipred','reverse','b_mb','aso','b_skip']
    indexed={tuple(c[k] for k in keys)+(c['mode'],):c for c in cases}
    changed={e:0 for e in ['cavlc','cabac0','cabac1','cabac2']}
    for c in cases:
        if c['mode']!=0:continue
        other=indexed[tuple(c[k] for k in keys)+(2,)]
        differs=(DEST/c['reference']).read_bytes()!=(DEST/other['reference']).read_bytes()
        if c['prediction']==1:assert not differs,'same-reference P/B_L0 boundary must have strength0'
        else:changed[c['entropy']]+=differs
    assert all(changed.values()),changed
    return changed


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional explicit all-plane cross-check only')
    args=parser.parse_args();cases=[]
    for entropy in ['cavlc','cabac0','cabac1','cabac2']:
        cabac=entropy!='cavlc';init=int(entropy[-1]) if cabac else 0
        for bipred in [0,2]:
            config=(cabac_configuration if cabac else configuration)(8,max_refs=3,bipred=bipred)
            for skip in [False,True]:
                for prediction in [0,1,2,3]:
                    for spatial in ([False,True] if prediction==0 else [True]):
                        for b_skip in ([False,True] if prediction==0 else [False]):
                            for reverse in [False,True]:
                                for b_mb in [0,1]:
                                    for aso in [False,True]:
                                        for mode in [0,1,2]:
                                            initial={b:smooth(b) for b in [False,True]};fields={b:[p[:] for p in ps] for b,ps in initial.items()}
                                            frames=[];packets=[];order=[True,False] if reverse else [False,True]
                                            def append(nals,idr=False):
                                                packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                                frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                            for i,bottom in enumerate(order):
                                                nals=[initial_pcm(bottom,reverse,a,initial[bottom],i==0) for a in [0,1]] if cabac else [pcm(bottom,i==0,reverse,initial[bottom])]
                                                append(nals,i==0)
                                            raw=bytearray(weave(initial))
                                            for bottom in order:
                                                nals=[]
                                                for mb in range(2):
                                                    pred=prediction if mb==b_mb else (-1 if skip else -2)
                                                    if cabac:n=cabac_inter(bottom,reverse,mb,mode,None,pred,None,spatial,init,skip=b_skip if mb==b_mb else False)
                                                    elif mb==b_mb:n=cavlc_b(bottom,reverse,mb,mode,None,pred,None,spatial,skip=b_skip)
                                                    else:n=cavlc_p(bottom,reverse,mb,mode,skip)
                                                    nals.append(n)
                                                append(nals[::-1] if aso else nals)
                                                l0=initial[bottom]
                                                l1=[initial[not bottom][0]]+[compensate(p,16,8,(0,2 if bottom else -2),True) for p in initial[not bottom][1:]]
                                                source=[p[:] for p in l0] if prediction==1 else ([p[:] for p in l1] if prediction==2 else
                                                    [[max(0,min(255,(-64*a+128*b+32)//64)) if bipred==2 and bottom==reverse else (a+b+1)//2 for a,b in zip(p,q)] for p,q in zip(l0,l1)])
                                                replace_mb(fields[bottom],source,b_mb)
                                                # No residual or motion differences inside either MB.
                                                # Across P/B: same L0 => bS0; L1/Bi/direct => bS1.
                                                strength=int(prediction!=1)
                                                fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},boundary_strengths={(1,0):strength})
                                                    for c,(p,w,h) in enumerate(zip(fields[bottom],[32,16,16],[16,8,8]))]
                                            raw.extend(weave(fields))
                                            for bottom in order:append([(cabac_follow if cabac else cavlc_follow)(bottom,reverse)])
                                            raw.extend(weave(initial))
                                            name=f'avc-mixed-pb-field-{entropy}-'+('skip' if skip else 'coded')+f'-pred{prediction}-spatial{int(spatial)}-weight{bipred}-'+('bottom-first' if reverse else 'top-first')+f'-b{b_mb}-mode{mode}'+('-aso' if aso else '')+('-bskip' if b_skip else '')
                                            (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                            (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                            cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,entropy=entropy,bipred=bipred,skip=skip,prediction=prediction,spatial=spatial,reverse=reverse,b_mb=b_mb,aso=aso,mode=mode,b_skip=b_skip))
    (DEST/'avc-mixed-pb-fields.json').write_text(json.dumps(dict(cases=cases,provenance='Original PCM pair, mixed non-reference P/B fields, retained P pair. Own scalar prediction, opposite-parity chroma correction, implicit weighting and weak strength0/1 boundary filtering. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed P/B field streams:',len(cases),'observable weak filter controls:',observable_controls(cases))
    if args.jm_decoder:verify_jm(cases,args.jm_decoder,all_planes=True)


if __name__=='__main__':main()
