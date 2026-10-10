#!/usr/bin/env python3
"""Original MBAFF SP chroma residual and frame/field fractional-motion matrix."""
import argparse,json
from pathlib import Path
from generate_avc_switching_mbaff_fixtures import DEST,configuration,source,slice_nal,reconstruct,mux


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--jm-decoder',type=Path);args=p.parse_args()
    config=configuration();cases=[]
    for topology,fields in [('frame',[False,False]),('field',[True,True]),('mixed',[False,True]),('reverse-mixed',[True,False])]:
        for kind in ['primary','secondary']:
            for chroma in [None,'dc','ac','both']:
                for mv in [(0,0),(1,3),(-3,-1),(4,2)]:
                    for reference_field in ([0,1] if any(fields) else [0]):
                        for reverse in [False,True]:
                            previous=source();gold=bytearray();frames=[];packets=[]
                            for frame,qs in enumerate([26,0,26,51]):
                                nals=[slice_nal(pair,fields[pair],frame,kind,qs,True,previous,'none',chroma,mv,reference_field,64) for pair in ([1,0] if reverse else [0,1])]
                                packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals);frames.append((frame,frame==0,packet));packets.append(packet.hex())
                                if frame:previous=reconstruct(previous,fields,kind,qs,True,'none',chroma,mv,reference_field,64)
                                gold.extend(bytes(v for plane in previous for v in plane))
                            name=f'avc-mbaff-sp-motion-chroma-{topology}-{kind}-{chroma or "none"}-x{mv[0]}-y{mv[1]}-ref{reference_field}'+('-aso' if reverse else '')
                            (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
                            cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=4,frame_picture=True,dc_level=64))
    (DEST/'avc-mbaff-sp-motion-chroma.json').write_text(json.dumps(dict(cases=cases,provenance='Original MBAFF PCM then primary/secondary SP frames. Frame/field/mixed pair geometry, QS0/26/51, signed luma and independent DC/AC chroma controls (escaped DC levels+/-64); frame/field AC scan, signed positive/negative fractional motion, same/opposite reference parity including chroma phase adjustment, reversed slice arrival. Independent scalar interpolation and switching YUV oracle. Deblocking disabled. No private samples, FFmpeg, external encoder or network. Explicit optional JM checks syntax/luma only due documented switching chroma differences.'),indent=2)+'\n')
    if args.jm_decoder:
        from generate_avc_switching_field_motion_fixtures import verify_jm
        verify_jm(cases,args.jm_decoder)

if __name__=='__main__':main()
