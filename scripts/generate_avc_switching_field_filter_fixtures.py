#!/usr/bin/env python3
"""Owned SP/SI field deblocking controls with scalar H.264 8.7 reference."""
import argparse
import json
from pathlib import Path
from generate_avc_switching_field_fixtures import (
    DEST, configuration, pcm, switching, reconstruct, weave, mux,
)


def smooth(bottom):
    return [[base+bottom*11+sign*(x//4*3+y//4*2)
             for y in range(height) for x in range(width)]
            for base,sign,width,height in [(64,1,32,16),(96,1,16,8),(160,-1,16,8)]]


def filter_plane(samples, width, height, chroma, mode, slices):
    """Fixed QP26/offset0 fixture oracle: alpha15, beta6, tc0(bS3)=1.

    Every MB is switching/intra: vertical external strength4, all other
    present edges strength3. No horizontal external edge in this geometry.
    """
    out=samples[:]
    if mode==1:return out
    size=8 if chroma else 16
    clip=lambda x:max(0,min(255,x))
    limit=lambda x,n:max(-n,min(n,x))
    for my in range(height//size):
        for mx in range(width//size):
            for vertical in [True,False]:
                for edge in range(0,size,4):
                    external=edge==0
                    if external and (mx==0 if vertical else my==0):continue
                    if external and mode==2 and slices==2:continue
                    strength=4 if external and vertical else 3
                    for line in range(size):
                        x=mx*size+(edge if vertical else line)
                        y=my*size+(line if vertical else edge)
                        at=y*width+x;step=1 if vertical else width
                        p=[out[at-(i+1)*step] for i in range(4)]
                        q=[out[at+i*step] for i in range(4)]
                        if abs(p[0]-q[0])>=15 or abs(p[1]-p[0])>=6 or abs(q[1]-q[0])>=6:continue
                        a=p[:];b=q[:];ap=abs(p[2]-p[0])<6;aq=abs(q[2]-q[0])<6
                        if strength==4:
                            strong=not chroma and abs(p[0]-q[0])<5
                            for near,far,dst,enabled in [(p,q,a,ap),(q,p,b,aq)]:
                                if strong and enabled:
                                    dst[0]=(near[2]+2*near[1]+2*near[0]+2*far[0]+far[1]+4)//8
                                    dst[1]=(near[2]+near[1]+near[0]+far[0]+2)//4
                                    dst[2]=(2*near[3]+3*near[2]+near[1]+near[0]+far[0]+4)//8
                                else:dst[0]=(2*near[1]+near[0]+far[1]+2)//4
                        else:
                            tc=2 if chroma else 1+ap+aq
                            delta=limit((4*(q[0]-p[0])+p[1]-q[1]+4)//8,tc)
                            a[0]=clip(p[0]+delta);b[0]=clip(q[0]-delta)
                            average=(p[0]+q[0]+1)//2
                            if not chroma:
                                if ap:a[1]+=limit((p[2]+average-2*p[1])//2,1)
                                if aq:b[1]+=limit((q[2]+average-2*q[1])//2,1)
                        for i in range(3):out[at-(i+1)*step]=a[i];out[at+i*step]=b[i]
    return out


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder',type=Path,help='Optional SP luma-only cross-check; SI excluded')
    args=parser.parse_args()
    config=configuration(8,max_refs=3);cases=[];changed={kind:0 for kind in ['primary','secondary','si']}
    for kind in changed:
        for coded in [False,True]:
            for reverse in [False,True]:
                for slices in [1,2]:
                    for qs in [0,26,51]:
                        for mode in [0,1,2]:
                            order=[True,False] if reverse else [False,True]
                            fields={b:smooth(b) for b in order};frames=[];packets=[];raw=bytearray()
                            def append(nals,idr):
                                packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                frames.append((len(frames),idr,packet));packets.append(packet.hex())
                            if kind!='si':
                                for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                raw.extend(weave(fields))
                            for i,bottom in enumerate(order):
                                idr=kind=='si' and i==0
                                append([switching(bottom,int(kind!='si'),qs,kind,coded,idr,reverse,
                                                  address,(0,0),mode)
                                        for address in ([None] if slices==1 else [0,1])],idr)
                                unfiltered=reconstruct(fields[bottom],qs,kind,coded,slices==2)
                                fields[bottom]=[filter_plane(p,w,h,c>0,mode,slices) for c,(p,w,h)
                                                in enumerate(zip(unfiltered,[32,16,16],[16,8,8]))]
                                changed[kind]+=fields[bottom]!=unfiltered
                            raw.extend(weave(fields))
                            name=f'avc-switching-field-filter-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-s{slices}-qs{qs}-mode{mode}'
                            (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                            (DEST/(name+'-reference.yuv')).write_bytes(raw)
                            cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                              configuration=config.hex(),packets=packets,frame_count=len(raw)//1536,
                                              kind=kind,coded=coded,reverse=reverse,slices=slices,qs=qs,mode=mode))
    assert all(changed.values()),changed
    controls={kind:dict(external=0,internal=0) for kind in changed}
    for case in cases:
        if case['mode']!=0:continue
        group=[v for v in cases if all(v[k]==case[k] for k in
               ['kind','coded','reverse','slices','qs'])]
        data={v['mode']:(DEST/v['reference']).read_bytes() for v in group}
        controls[case['kind']]['internal']+=data[2]!=data[1]
        if case['slices']==2:controls[case['kind']]['external']+=data[0]!=data[2]
    assert all(all(v.values()) for v in controls.values()),controls
    (DEST/'avc-switching-field-filter.json').write_text(json.dumps(dict(cases=cases,changed=changed,
        provenance='Original compact complementary 32x16 fields: smooth I_PCM pair then primary/secondary SP, or SI field pair; one/two slices, modes0/1/2, QS0/26/51 and zero/signed residual. Own scalar switching matrix followed by fixed QP26 H.264 8.7 deblocking oracle. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
    print('Filtered field controls with changed pixels:',changed,controls)
    if args.jm_decoder:
        from generate_avc_switching_field_motion_fixtures import verify_jm
        verify_jm([c for c in cases if c['kind']!='si'],args.jm_decoder)


if __name__=='__main__':main()
