#!/usr/bin/env python3
"""Original two-row complementary SP/SI fields with scalar deblocking oracle."""
import argparse
import json
from pathlib import Path
from generate_avc_field_rows_samples import configuration
from generate_avc_switching_field_fixtures import DEST, header, switching, reconstruct, weave, mux
from generate_avc_switching_field_filter_fixtures import filter_plane
from generate_avc_switching_field_motion_fixtures import verify_jm


def smooth(bottom):
    return [[base+bottom*11+sign*(x//4*3+y//4*2)
             for y in range(height) for x in range(width)]
            for base,sign,width,height in [(64,1,32,32),(96,1,16,16),(160,-1,16,16)]]


def pcm(bottom, idr, reverse, source):
    bits=header(bottom,0,0,'pcm',idr,reverse)
    for mb in range(4):
        bits.ue(25);bits.align()
        for plane,width,size in zip(source,[32,16,16],[16,8,8]):
            for row in range(size):
                for col in range(size):
                    bits.u(plane[(mb//2*size+row)*width+mb%2*size+col],8)
    return bits.nal(0x65 if idr else 0x41)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional SP luma cross-check only')
    args=parser.parse_args();config=configuration(8);cases=[]
    strength_controls={kind:0 for kind in ['primary','secondary','si']}
    layouts={'one':[[0,1,2,3]],'rows':[[0,1],[2,3]],'blocks':[[0],[1],[2],[3]]}
    for kind in ['primary','secondary','si']:
        for coded in [False,True]:
            for reverse in [False,True]:
                for layout,groups in layouts.items():
                    owners={address:i for i,group in enumerate(groups) for address in group}
                    for aso in ([False] if layout=='one' else [False,True]):
                        for qs in [0,26,51]:
                            for mode in [0,1,2]:
                                order=[True,False] if reverse else [False,True]
                                fields={b:smooth(b) for b in order};frames=[];packets=[];raw=bytearray()
                                def append(nals,idr):
                                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                    frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                if kind!='si':
                                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                    raw.extend(weave(fields,32))
                                for i,bottom in enumerate(order):
                                    idr=kind=='si' and i==0
                                    nals=[switching(bottom,int(kind!='si'),qs,kind,coded,idr,reverse,
                                                    deblock=mode,addresses=group) for group in groups]
                                    append(nals[::-1] if aso else nals,idr)
                                    unfiltered=reconstruct(fields[bottom],qs,kind,coded,height=32,slice_groups=groups)
                                    fields[bottom]=[filter_plane(p,w,h,c>0,mode,len(groups),owners) for c,(p,w,h)
                                                    in enumerate(zip(unfiltered,[32,16,16],[32,16,16]))]
                                    if mode==0 and layout=='one':
                                        wrong=[filter_plane(p,w,h,c>0,mode,len(groups),owners,horizontal_strength=4)
                                               for c,(p,w,h) in enumerate(zip(unfiltered,[32,16,16],[32,16,16]))]
                                        strength_controls[kind]+=wrong!=fields[bottom]
                                raw.extend(weave(fields,32))
                                name=f'avc-switching-field-rows-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-{layout}-qs{qs}-mode{mode}'+('-aso' if aso else '')
                                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,64,60))
                                (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                    configuration=config.hex(),packets=packets,frame_count=len(raw)//3072,
                                    width=32,height=64,kind=kind,coded=coded,reverse=reverse,layout=layout,aso=aso,qs=qs,mode=mode))
    assert all(strength_controls.values()),strength_controls
    (DEST/'avc-switching-field-rows.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original 32x32 compact fields woven into32x64 frames. Primary/secondary SP and SI, zero/signed residual, both parity orders, one/row/block slices and reversed slice arrival, QS0/26/51 and deblocking0/1/2. Scalar spatial DC, field coefficient scan, switching matrix and QP26 filtering; horizontal external field boundaries use strength3. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Two-row switching field streams:',len(cases),'observable field strength controls:',strength_controls)
    if args.jm_decoder:verify_jm([c for c in cases if c['kind']!='si'],args.jm_decoder)


if __name__=='__main__':main()
