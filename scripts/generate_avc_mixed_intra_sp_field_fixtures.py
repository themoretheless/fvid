#!/usr/bin/env python3
"""Original mixed I/SP fields: PCM, intra4 DC and intra16 DC companions."""
import argparse
import json
from pathlib import Path
from generate_avc_switching_field_fixtures import DEST, configuration, pcm, switching, reconstruct, weave, mux, header
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane
from generate_avc_switching_field_motion_fixtures import verify_jm


def replace_mb(destination, source, mb):
    for target,previous,width,size in zip(destination,source,[32,16,16],[16,8,8]):
        for row in range(size):
            start=row*width+mb*size
            target[start:start+size]=previous[start:start+size]


def backdrop(bottom, intra_kind):
    result=smooth(bottom)
    if intra_kind!='pcm':
        # Close to ordinary intra DC128 so external filtering changes pixels.
        result=[[v+shift for v in plane] for plane,shift in zip(result,[56,24,-24])]
    return result


def intra(bottom, reverse, address, mode, kind, source):
    bits=header(bottom,1,0,'pcm',False,reverse,address,mode)
    if kind=='pcm':
        bits.ue(25);bits.align()
        for plane,width,size in zip(source,[32,16,16],[16,8,8]):
            for row in range(size):
                for col in range(size):bits.u(plane[row*width+address*size+col],8)
    elif kind=='i4':
        bits.ue(0)
        for _ in range(16):bits.u(1)
        bits.ue(0);bits.ue(3)
    else:
        bits.ue(3);bits.ue(0);bits.se(0);bits.u(1)
    return bits.nal(0x41)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional luma cross-check only')
    args=parser.parse_args();config=configuration(8,max_refs=3);cases=[]
    for intra_kind in ['pcm','i4','i16']:
        for kind in ['primary','secondary']:
            for coded in [False,True]:
                for reverse in [False,True]:
                    for sp_mb in [0,1]:
                        for aso in [False,True]:
                            for qs in [0,26,51]:
                                for mode in [0,1,2]:
                                    fields={b:backdrop(b,intra_kind) for b in [False,True]};frames=[];packets=[]
                                    order=[True,False] if reverse else [False,True]
                                    def append(nals,idr=False):
                                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                        frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                    raw=bytearray(weave(fields))
                                    for bottom in order:
                                        intra_source=[[v+7 for v in p] for p in smooth(bottom)] if intra_kind=='pcm' else [[128]*512,[128]*128,[128]*128]
                                        nals=[switching(bottom,1,qs,kind,coded,False,reverse,deblock=mode,addresses=[mb]) if mb==sp_mb
                                              else intra(bottom,reverse,mb,mode,intra_kind,intra_source) for mb in range(2)]
                                        append(nals[::-1] if aso else nals)
                                        unfiltered=reconstruct(fields[bottom],qs,kind,coded)
                                        replace_mb(unfiltered,intra_source,1-sp_mb)
                                        fields[bottom]=[filter_plane(p,w,h,c>0,(2 if mode!=1 else 1) if intra_kind=='pcm' else mode,2,owners={0:0,1:1},
                                                                    switching_blocks={sp_mb} if intra_kind=='pcm' else None) for c,(p,w,h)
                                                        in enumerate(zip(unfiltered,[32,16,16],[16,8,8]))]
                                    raw.extend(weave(fields))
                                    # Subsequent whole P field validates retained mixed-picture references.
                                    for bottom in order:append([switching(bottom,2,26,'plain',False,False,reverse)])
                                    raw.extend(weave(fields))
                                    name=f'avc-mixed-intra-sp-field-{intra_kind}-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-sp{sp_mb}-qs{qs}-mode{mode}'+('-aso' if aso else '')
                                    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                    (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                    cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                        configuration=config.hex(),packets=packets,frame_count=3,kind=kind,coded=coded,
                                        reverse=reverse,sp_mb=sp_mb,aso=aso,qs=qs,mode=mode,intra_kind=intra_kind))
    controls={'i4':0,'i16':0}
    paired={tuple(c[k] for k in ['kind','coded','reverse','sp_mb','aso','qs','intra_kind','mode']):c for c in cases}
    for case in cases:
        if case['intra_kind']=='pcm' or case['mode']!=0:continue
        key=tuple(case[k] for k in ['kind','coded','reverse','sp_mb','aso','qs','intra_kind'])+(2,)
        controls[case['intra_kind']]+=(DEST/case['reference']).read_bytes()!=(DEST/paired[key]['reference']).read_bytes()
    assert all(controls.values()),controls
    (DEST/'avc-mixed-intra-sp-fields.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original smooth PCM field pair, mixed I/SP field pair with I_PCM fresh gradient or ordinary intra4/intra16 DC128, then P-skip pair using retained references. Primary/secondary SP, zero/signed switching residual, both positions and parity orders, normal/reversed NAL arrival, QS0/26/51 and filter0/1/2. Ordinary intra companions start nearDC128 to exercise external filtering. Scalar switching and deblocking; PCM QP0 prevents filtering on averageQP13 boundary, ordinary intra QP26. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed I/SP field streams:',len(cases),'observable external filter controls:',controls)
    if args.jm_decoder:verify_jm(cases,args.jm_decoder)


if __name__=='__main__':main()
