#!/usr/bin/env python3
"""Original MBAFF SP streams and independent scalar switching reconstruction."""
import argparse
import json
from pathlib import Path
from generate_avc_field_pcm_samples import Writer
from generate_avc_switching_luma_fixtures import DEST, reference
from generate_avc_switching_chroma_fixtures import chroma as primary_chroma
from generate_avc_secondary_sp_fixtures import chroma as secondary_chroma
from avc_fixture_mp4 import mux


def configuration():
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0)
    b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0)
    b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0)
    pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps


def source():
    return [[48+(x*5+y*3)%128 for y in range(32) for x in range(32)],
            [64+x*5+y*3 for y in range(16) for x in range(16)],
            [192-x*5-y*3 for y in range(16) for x in range(16)]]


def indices(pair,parity,field,component):
    side=8 if component else 16; width=side*2
    origin=parity if field else parity*side;step=2 if field else 1
    return [(origin+y*step)*width+pair*side+x for y in range(side) for x in range(side)]


def write_chroma_dc_one(b,negative,level=1):
    """Table9-5 coeff_token, escaped level (9.2.2), total_zeros1."""
    if level==1:b.u(1);b.u(int(negative)) # TC1/T1
    elif level==64:
        b.u(7,6) # TC1/T0
        b.u(1,16);b.u(94+int(negative),12) # prefix15, 12-bit suffix
    else:raise ValueError('fixture DC level must be1 or64')
    b.u(1,2)


def slice_nal(pair,field,frame,kind,qs,coded,previous,skip,chroma=None,mv=(0,0),reference_field=0,dc_level=1):
    b=Writer();b.ue(pair);b.ue(2 if frame==0 else 3);b.ue(0);b.u(frame,4);b.u(0)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if frame:b.u(0);b.u(0)
    if frame==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0)
    if frame:b.u(int(kind=='secondary'));b.se(qs-26)
    b.ue(1)
    if frame and skip=='both':b.ue(2);return b.nal(0x41)
    for parity in [0,1]:
        if frame:
            if skip=='top' and parity==0:b.ue(1);continue
            if skip=='bottom' and parity==1:b.ue(1);continue
            if not (skip=='top' and parity==1):b.ue(0)
        if parity==0 or frame and skip=='top':b.u(int(field))
        if frame==0:
            b.ue(25);b.align()
            for component in range(3):
                for i in indices(pair,parity,field,component):b.u(previous[component][i],8)
        else:
            b.ue(0)
            if field:b.u(1-reference_field) # te(v), two field references
            # A coded bottom frame MB predicts the top MB vector; field parity
            # has no same-field top neighbor in this one-row independent slice.
            b.se(mv[0] if parity==0 or field else 0);b.se(mv[1] if parity==0 or field else 0)
            b.ue((12 if chroma in ['ac','both'] else (19 if chroma=='dc' else 11)) if coded else 0)
            if coded:
                b.se(0)
                for block in range(16):b.u(1,2);b.u((block+pair*2+parity)%2);b.u(1)
                if chroma:
                    for component in range(2):
                        if chroma in ['dc','both']:write_chroma_dc_one(b,(component+pair*2+parity)%2,dc_level)
                        else:b.u(1,2) # chroma DC TotalCoeff0
                    if chroma in ['ac','both']:
                        for component in range(2):
                            for block in range(4):b.u(1,2);b.u((block+component+pair*2+parity)%2);b.u(1)
    return b.nal(0x65 if frame==0 else 0x41)


def reconstruct(previous,fields,kind,qs,coded,skip,chroma=None,mv=(0,0),reference_field=0,dc_level=1):
    out=[p.copy() for p in previous]
    # A separate scalar interpolator operates on complete frame/field reference
    # planes before extracting the current logical macroblock.
    from generate_avc_switching_field_motion_fixtures import compensate
    predictions={}
    def prediction(component,pair,parity,field):
        if mv==(0,0) and (not field or reference_field==0):
            return [previous[component][i] for i in indices(pair,parity,field,component)]
        ref_parity=parity^reference_field if field else None
        key=(component,ref_parity)
        width=32 if component==0 else 16
        if key not in predictions:
            plane=previous[component]
            if field:plane=[v for y in range(ref_parity,width,2) for v in plane[y*width:(y+1)*width]]
            motion=list(mv)
            if component and field and ref_parity!=parity:motion[1]+=2 if parity else -2
            predictions[key]=compensate(plane,width,width//2 if field else width,tuple(motion),component!=0)
        side=16 if component==0 else 8;origin_y=0 if field else parity*side
        return [predictions[key][(origin_y+y)*width+pair*side+x] for y in range(side) for x in range(side)]
    for pair,field in enumerate(fields):
        for parity in [0,1]:
            locations=indices(pair,parity,field,0);p=prediction(0,pair,parity,field)
            for block in range(16):
                bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
                selected=[(by*4+y)*16+bx*4+x for y in range(4) for x in range(4)]
                residual=[(1 if (block+pair*2+parity)%2==0 else -1) if coded and skip not in ["both", "top" if parity==0 else "bottom"] else 0]+[0]*15
                values=reference([p[i] for i in selected],residual,26,qs,kind=='secondary')
                for i,v in zip(selected,values):out[0][locations[i]]=v
            for component in [1,2]:
                locations=indices(pair,parity,field,component);p=prediction(component,pair,parity,field)
                qsc=39 if qs==51 else qs
                dc=[0]*4;ac=[[0]*16 for _ in range(4)]
                active=coded and skip not in ['both','top' if parity==0 else 'bottom']
                if active and chroma in ['dc','both']:dc[2]=dc_level if (component-1+pair*2+parity)%2==0 else -dc_level
                if active and chroma in ['ac','both']:
                    for block in range(4):ac[block][4 if field else 1]=1 if (block+component-1+pair*2+parity)%2==0 else -1
                values=primary_chroma(p,dc,ac,26,qsc) if kind=='primary' else secondary_chroma(p,dc,ac,qsc)
                for i,v in zip(locations,values):out[component][i]=v
    return out


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path)
    args=parser.parse_args();config=configuration();cases=[]
    for topology,fields in [('frame',[False,False]),('field',[True,True]),('mixed',[False,True]),('reverse-mixed',[True,False])]:
        for kind in ['primary','secondary']:
            for coded in [False,True]:
                for reverse in [False,True]:
                    for skip in ['none','top','bottom','both']:
                        previous=source();frames=[];packets=[];gold=bytearray()
                        for frame,qs in enumerate([26,0,26,51]):
                            nals=[slice_nal(pair,False if frame and skip=="both" else fields[pair],frame,kind,qs,coded,previous,skip) for pair in ([1,0] if reverse else [0,1])]
                            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                            frames.append((frame,frame==0,packet));packets.append(packet.hex())
                            if frame:previous=reconstruct(previous,[False,False] if skip=="both" else fields,kind,qs,coded,skip)
                            gold.extend(bytes(v for plane in previous for v in plane))
                        name=f'avc-switching-mbaff-{topology}-{kind}-'+('signed' if coded else 'zero')+('-aso' if reverse else '')+('' if skip=='none' else '-skip-'+skip)
                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                        (DEST/(name+'-reference.yuv')).write_bytes(gold)
                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=4,frame_picture=True))
    (DEST/'avc-switching-mbaff.json').write_text(json.dumps(dict(cases=cases,provenance='Original 32x32 MBAFF PCM then primary/secondary SP frames, frame/field/mixed pair layouts, independent slices and reversed arrival; QS0/26/51, zero and signed luma DC residual, coded and top/bottom/both skip variants (whole skipped independent pairs infer frame mode). Independent scalar luma and chroma switching oracle, deblocking disabled. No private media, external encoder, FFmpeg or network. Optional explicit JM verifies syntax and luma only.'),indent=2)+'\n')
    if args.jm_decoder:
        from generate_avc_switching_field_motion_fixtures import verify_jm
        verify_jm(cases,args.jm_decoder)

if __name__=='__main__':main()
