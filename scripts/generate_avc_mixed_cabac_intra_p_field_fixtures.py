#!/usr/bin/env python3
"""Original mixed CABAC I/P complementary fields with own scalar pixels."""
import argparse
import json
from pathlib import Path
from generate_avc_field_cabac_samples import configuration
from generate_avc_switching_field_fixtures import DEST, weave, mux
from generate_avc_mixed_intra_sp_field_fixtures import replace_mb, backdrop
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import verify_jm


from generate_avc_mixed_cabac_intra_b_field_fixtures import initial_pcm, slice_nal, follow_p, observable_filter_controls


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional explicit all-plane cross-check only')
    args=parser.parse_args()
    cases=[]
    for init in range(3):
        for bipred in [0]:
            config=configuration(8,max_refs=3,bipred=bipred)
            for intra_kind in ['pcm','i4','i16']:
                for prediction in [-2,-1]:
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
                                            # Both skip and coded P_L0 use the initial same-parity field.
                                            fields[bottom]=[p[:] for p in initial[bottom]]
                                            replace_mb(fields[bottom],source,1-b_mb)
                                            if intra_kind!='pcm':
                                                fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},switching_blocks={1-b_mb})
                                                    for c,(p,w,h) in enumerate(zip(fields[bottom],[32,16,16],[16,8,8]))]
                                        raw.extend(weave(fields))
                                        # B/I picture is non-reference: P must still use the initial PCM pair.
                                        for bottom in order:append([follow_p(bottom,reverse)])
                                        raw.extend(weave(initial))
                                        name=f'avc-mixed-cabac-intra-p-field-{intra_kind}-pred{prediction}-'+('bottom-first' if reverse else 'top-first')+f'-b{b_mb}-mode{mode}-weight{bipred}-spatial{int(spatial)}-init{init}'+('-aso' if aso else '')
                                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                        (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,intra_kind=intra_kind,prediction=prediction,reverse=reverse,b_mb=b_mb,aso=aso,mode=mode,bipred=bipred,spatial=spatial,init=init))
    (DEST/'avc-mixed-cabac-intra-p-fields.json').write_text(json.dumps(dict(cases=cases,provenance='Original PCM fields, mixed non-reference CABAC I/P fields, subsequent P fields. Own zero-motion prediction and scalar deblocking. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed CABAC I/P field streams:',len(cases))
    print('Observable external filtering controls:',observable_filter_controls(cases))
    if args.jm_decoder:verify_jm(cases,args.jm_decoder,all_planes=True)


if __name__=='__main__':main()
