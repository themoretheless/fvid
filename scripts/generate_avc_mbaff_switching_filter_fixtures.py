#!/usr/bin/env python3
"""Original two-row MBAFF SP filter fixtures; independent physical ownership oracle."""
import json
from generate_avc_switching_mbaff_fixtures import DEST, configuration, indices, slice_nal, reconstruct
from avc_fixture_mp4 import mux


def filter_picture(planes, fields, mode, offsets=(0,0), slices=None):
    """H.264 8.7 at QP26, with selected signed alpha/beta offsets.

    Ownership is obtained by painting each macroblock's samples, including field
    parity, rather than using the decoder's edge traversal or address mapping.
    Default slices are independent pairs; explicit identities cover whole pictures.
    """
    # Table8-16/17 at QP26; actual offsets are twice slice syntax.
    alpha={-4:9,0:15,4:25}[offsets[0]]
    beta={-2:4,0:6,2:7}[offsets[1]]
    tc0={-4:1,0:1,4:2}[offsets[0]]
    if slices is None:slices=[address//2 for address in range(2*len(fields))]
    result=[]
    for component, plane in enumerate(planes):
        out=plane[:];width=16 if component else 32;size=width//2
        owners=[None]*len(out)
        for pair,field in enumerate(fields):
            for parity in range(2):
                for point in indices(pair,parity,field,component):owners[point]=2*pair+parity
        assert all(v is not None for v in owners)
        if mode==1:result.append(out);continue
        for address in range(2*len(fields)):
            pair,parity=divmod(address,2);field=fields[pair]
            locations=indices(pair,parity,field,component);origin=locations[0]
            ox,oy=origin%width,origin//width;rowstep=2 if field else 1
            for vertical in [True,False]:
                for edge in range(0,size,4):
                    external=edge==0
                    if vertical and external and ox==0:continue
                    # A frame MB below a field pair has two external horizontal
                    # boundaries, one per physical parity, with stride two.
                    mixed=not vertical and external and not field and oy>0 and fields[owners[(oy-1)*width+ox]//2]
                    for extra in range(2 if mixed else 1):
                        for line in range(size):
                            x=ox+(edge if vertical else line)
                            y=oy+(line*rowstep if vertical else edge*rowstep)+extra
                            step=1 if vertical else width*(2 if mixed else rowstep)
                            at=y*width+x
                            if at-step<0:continue
                            neighbour=owners[at-step]
                            if external and mode==2 and slices[neighbour]!=slices[address]:continue
                            strength=4 if external and (vertical or not field and not fields[neighbour//2]) else 3
                            p=[out[at-(i+1)*step] for i in range(4)]
                            q=[out[at+i*step] for i in range(4)]
                            if abs(p[0]-q[0])>=alpha or abs(p[1]-p[0])>=beta or abs(q[1]-q[0])>=beta:continue
                            a=p[:];b=q[:];ap=abs(p[2]-p[0])<beta;aq=abs(q[2]-q[0])<beta
                            if strength==4:
                                strong=not component and abs(p[0]-q[0])<alpha//4+2
                                for near,far,dst,enabled in [(p,q,a,ap),(q,p,b,aq)]:
                                    if strong and enabled:
                                        dst[0]=(near[2]+2*near[1]+2*near[0]+2*far[0]+far[1]+4)//8
                                        dst[1]=(near[2]+near[1]+near[0]+far[0]+2)//4
                                        dst[2]=(2*near[3]+3*near[2]+near[1]+near[0]+far[0]+4)//8
                                    else:dst[0]=(2*near[1]+near[0]+far[1]+2)//4
                            else:
                                limit=lambda v,n:max(-n,min(n,v))
                                tc=tc0+1 if component else tc0+ap+aq
                                delta=limit((4*(q[0]-p[0])+p[1]-q[1]+4)//8,tc)
                                a[0]=max(0,min(255,p[0]+delta));b[0]=max(0,min(255,q[0]-delta))
                                average=(p[0]+q[0]+1)//2
                                if not component:
                                    if ap:a[1]+=limit((p[2]+average-2*p[1])//2,tc0)
                                    if aq:b[1]+=limit((q[2]+average-2*q[1])//2,tc0)
                            for i in range(3):out[at-(i+1)*step]=a[i];out[at+i*step]=b[i]
        result.append(out)
    return result


def main():
    layouts={'frame':[False]*4,'field':[True]*4,'checker':[False,True,True,False],
             'reverse-checker':[True,False,False,True],'field-above-frame':[True,True,False,False],
             'frame-above-field':[False,False,True,True]}
    cases=[];controls={};config=configuration(64)
    for topology,fields in layouts.items():
        for kind in ['primary','secondary']:
            for coded in [False,True]:
                for reverse in [False,True]:
                    for mode in [0,1,2]:
                        previous=[[base+sign*(x//4*3+y//4*2)+(y%2)*2 for y in range(h) for x in range(w)]
                                  for base,sign,w,h in [(64,1,32,64),(96,1,16,32),(160,-1,16,32)]]
                        frames=[];packets=[];gold=bytearray()
                        for frame,qs in enumerate([26,0,26,51]):
                            nals=[slice_nal(pair,fields[pair],frame,kind,qs,coded,previous,'none',mode=mode if frame else 1)
                                  for pair in (range(3,-1,-1) if reverse else range(4))]
                            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                            frames.append((frame,frame==0,packet));packets.append(packet.hex())
                            if frame:previous=filter_picture(reconstruct(previous,fields,kind,qs,coded,'none'),fields,mode)
                            gold.extend(bytes(v for plane in previous for v in plane))
                        key=(topology,kind,coded,reverse);controls.setdefault(key,{})[mode]=bytes(gold)
                        name=f'avc-mbaff-switching-filter-{topology}-{kind}-'+('signed' if coded else 'zero')+('-aso' if reverse else '')+f'-mode{mode}'
                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,64,60))
                        (DEST/(name+'-reference.yuv')).write_bytes(gold)
                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=4,frame_picture=True,width=32,height=64))
    observable={kind:{'internal':0,'external':0} for kind in ['primary','secondary']}
    for (_,kind,_,_),data in controls.items():
        observable[kind]['internal']+=data[2]!=data[1]
        observable[kind]['external']+=data[0]!=data[2]
    assert all(all(v.values()) for v in observable.values()),observable
    (DEST/'avc-mbaff-switching-filter.json').write_text(json.dumps(dict(cases=cases,observable=observable,
        provenance='Original 32x64 PCM then primary/secondary SP, six two-row frame/field layouts, QS0/26/51, zero/signed residual, independent pair slices and ASO, filter modes0/1/2. Own physical ownership and scalar H.264 8.7 filter oracle, including double horizontal field-to-frame boundary. No private media, FFmpeg, external codec or network.'),indent=2)+'\n')
    print(len(cases),observable)

if __name__=='__main__':main()
