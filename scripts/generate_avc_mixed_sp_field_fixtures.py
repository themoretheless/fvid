#!/usr/bin/env python3
"""Original mixed P/SP compact fields with independent switching/filter oracle."""
import argparse
import json
from pathlib import Path
from generate_avc_switching_field_fixtures import DEST, configuration, pcm, switching, reconstruct, weave, mux
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import verify_jm


def replace_mb(destination, source, mb):
    for target,previous,width,size in zip(destination,source,[32,16,16],[16,8,8]):
        for row in range(size):
            start=row*width+mb*size
            target[start:start+size]=previous[start:start+size]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional luma cross-check only')
    args=parser.parse_args();config=configuration(8,max_refs=3);cases=[]
    for kind in ['primary','secondary']:
        for coded in [False,True]:
            for reverse in [False,True]:
                for sp_mb in [0,1]:
                    for aso in [False,True]:
                        for qs in [0,26,51]:
                            for mode in [0,1,2]:
                                fields={b:smooth(b) for b in [False,True]};frames=[];packets=[]
                                order=[True,False] if reverse else [False,True]
                                def append(nals,idr=False):
                                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                    frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                raw=bytearray(weave(fields))
                                for bottom in order:
                                    nals=[switching(bottom,1,qs,kind if mb==sp_mb else 'plain',
                                                    coded if mb==sp_mb else False,False,reverse,
                                                    deblock=mode,addresses=[mb]) for mb in range(2)]
                                    append(nals[::-1] if aso else nals)
                                    unfiltered=reconstruct(fields[bottom],qs,kind,coded)
                                    replace_mb(unfiltered,fields[bottom],1-sp_mb)
                                    fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},
                                                                switching_blocks={sp_mb}) for c,(p,w,h)
                                                    in enumerate(zip(unfiltered,[32,16,16],[16,8,8]))]
                                raw.extend(weave(fields))
                                # Subsequent whole P field validates retained mixed-picture references.
                                for bottom in order:append([switching(bottom,2,26,'plain',False,False,reverse)])
                                raw.extend(weave(fields))
                                name=f'avc-mixed-sp-field-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-sp{sp_mb}-qs{qs}-mode{mode}'+('-aso' if aso else '')
                                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                    configuration=config.hex(),packets=packets,frame_count=3,kind=kind,coded=coded,
                                    reverse=reverse,sp_mb=sp_mb,aso=aso,qs=qs,mode=mode))
    (DEST/'avc-mixed-sp-fields.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original smooth PCM field pair, mixed P-skip/SP field pair with one macroblock per slice, then ordinary P-skip pair using retained references. Primary/secondary SP, zero/signed switching residual, both SP positions and parity orders, normal/reverse NAL arrival, QS0/26/51, filter0/1/2. Scalar switching and field filtering preserve unquantized P skips. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed P/SP field streams:',len(cases))
    if args.jm_decoder:
        # JM applies q-side slice type at a P-right/SP-left external edge,
        # unlike H.264 8.7.2.1's explicit either-side switching condition.
        selected=[c for c in cases if c['mode']!=0 or c['sp_mb']==1]
        verify_jm(selected,args.jm_decoder)
        print('Normative-only P-right/SP-left cross-slice controls:',len(cases)-len(selected))


if __name__=='__main__':main()
