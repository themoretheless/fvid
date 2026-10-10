#!/usr/bin/env python3
"""Original MBAFF SI and ordinary intra neighbors with scalar sample availability."""
import json
from generate_avc_switching_mbaff_fixtures import Writer, DEST, indices, write_chroma_dc_one
from generate_avc_switching_luma_fixtures import reference
from generate_avc_secondary_sp_fixtures import chroma
from avc_fixture_mp4 import mux


def configuration(constrained):
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(3);b.u(0);b.ue(1);b.ue(0)
    b.u(0);b.u(1);b.u(1);b.u(0);b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0)
    b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(int(constrained));b.u(0);pps=b.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps


def slice_nal(addresses,fields,switching,frame,qs,coded,pcm,chroma_mode=None,dc_level=1):
    b=Writer();b.ue(addresses[0]//2);b.ue(4);b.ue(0);b.u(frame,4);b.u(0)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if frame==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.se(qs-26);b.ue(1)
    counts=[None]*1024
    chroma_counts=[[None]*256 for _ in range(2)]
    for address in addresses:
        if address%2==0:b.u(int(fields[address//2]))
        if address in pcm:
            b.ue(26);b.align()
            for component,side in [(0,16),(1,8),(2,8)]:
                for _ in range(side*side):b.u(pcm_value(address,component),8)
            for pos in indices(address//2,address%2,fields[address//2],0):counts[pos]=16
            for component in range(2):
                for pos in indices(address//2,address%2,fields[address//2],component+1):chroma_counts[component][pos]=16
            continue
        b.ue(0 if switching[address] else 1)
        for _ in range(16):b.u(1) # predicted intra4 DC mode
        b.ue(0);active=coded and switching[address]
        b.ue((0 if chroma_mode in ['ac','both'] else (1 if chroma_mode=='dc' else 2)) if active else 3)
        if active:b.se(0)
        locations=indices(address//2,address%2,fields[address//2],0)
        step=2 if fields[address//2] else 1
        for block in range(16):
            bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
            pos=locations[by*4*16+bx*4];y,x=divmod(pos,32)
            if active:
                neighbours=[counts[yy*32+xx] for xx,yy in [(x-1,y),(x,y-step)] if 0<=xx<32 and 0<=yy<32 and counts[yy*32+xx] is not None]
                nc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 0
                # Table9-5, TotalCoeff1/TrailingOnes1; PCM contributes count16.
                token,length=(1,2) if nc<2 else ((2,2) if nc<4 else ((14,4) if nc<8 else (1,6)))
                b.u(token,length);b.u((block+address)%2);b.u(1)
            for yy in range(4):
                for xx in range(4):counts[locations[(by*4+yy)*16+bx*4+xx]]=int(active)
        if active and chroma_mode:
            for component in range(2):
                if chroma_mode in ['dc','both']:write_chroma_dc_one(b,(component+address)%2,dc_level)
                else:b.u(1,2)
        for component in range(2):
            locations=indices(address//2,address%2,fields[address//2],component+1)
            for block in range(4):
                bx=block%2;by=block//2;pos=locations[by*4*8+bx*4];y,x=divmod(pos,16)
                ac=active and chroma_mode in ['ac','both']
                if ac:
                    counts_c=chroma_counts[component]
                    neighbours=[counts_c[yy*16+xx] for xx,yy in [(x-1,y),(x,y-step)] if 0<=xx<16 and 0<=yy<16 and counts_c[yy*16+xx] is not None]
                    nc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 0
                    token,length=(1,2) if nc<2 else ((2,2) if nc<4 else ((14,4) if nc<8 else (1,6)))
                    b.u(token,length);b.u((block+component+address)%2);b.u(1)
                for yy in range(4):
                    for xx in range(4):chroma_counts[component][locations[(by*4+yy)*8+bx*4+xx]]=int(ac)
    return b.nal(0x65 if frame==0 else 0x41)


def pcm_value(address,component):
    return [96+address*7,64+address*19,192-address*17][component]


def reconstruct(fields,switching,qs,coded,constrained,groups,pcm,chroma_mode=None,dc_level=1):
    out=[[0]*1024,[0]*256,[0]*256]
    for addresses in groups:
        ready=[[0]*1024,[0]*256,[0]*256]
        for address in addresses:
            pair,parity=divmod(address,2);field=fields[pair];step=2 if field else 1
            si=switching[address];tag=2 if si else 1
            if address in pcm:
                for component in range(3):
                    for pos in indices(pair,parity,field,component):out[component][pos]=pcm_value(address,component);ready[component][pos]=1
                continue
            def edge(component,coordinates):
                width=32 if component==0 else 16;height=width
                if not all(0<=x<width and 0<=y<height and ready[component][y*width+x] and not (constrained and not si and ready[component][y*width+x]==2) for x,y in coordinates):return []
                return [out[component][y*width+x] for x,y in coordinates]
            locations=indices(pair,parity,field,0)
            for block in range(16):
                bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2
                pos=locations[by*4*16+bx*4];y,x=divmod(pos,32)
                top=edge(0,[(x+i,y-step) for i in range(4)])
                left=edge(0,[(x-1,y+i*step) for i in range(4)])
                values=top+left;dc=(sum(values)+len(values)//2)//len(values) if values else 128
                levels=[(1 if (block+address)%2==0 else -1) if coded and si else 0]+[0]*15
                pixels=reference([dc]*16,levels,26,qs,True) if si else [dc]*16
                for i,v in enumerate(pixels):
                    pos=locations[(by*4+i//4)*16+bx*4+i%4];out[0][pos]=v;ready[0][pos]=tag
            for component in [1,2]:
                locations=indices(pair,parity,field,component);y,x=divmod(locations[0],16)
                tops=[edge(component,[(x+half*4+i,y-step) for i in range(4)]) for half in range(2)]
                lefts=[edge(component,[(x-1,y+(half*4+i)*step) for i in range(4)]) for half in range(2)]
                prediction=[]
                for row in range(8):
                    for col in range(8):
                        ts=tops[col//4];ls=lefts[row//4]
                        values=(ts or ls) if row<4 and col>=4 else ((ls or ts) if row>=4 and col<4 else ts+ls)
                        prediction.append((sum(values)+len(values)//2)//len(values) if values else 128)
                dc=[0]*4;ac=[[0]*16 for _ in range(4)]
                if coded and si and chroma_mode in ['dc','both']:dc[2]=dc_level if (component-1+address)%2==0 else -dc_level
                if coded and si and chroma_mode in ['ac','both']:
                    for block in range(4):ac[block][4 if field else 1]=1 if (block+component-1+address)%2==0 else -1
                values=chroma(prediction,dc,ac,39 if qs==51 else qs) if si else prediction
                for pos,v in zip(locations,values):out[component][pos]=v;ready[component][pos]=tag
    return bytes(v for plane in out for v in plane)


def main():
    cases=[]
    patterns={'all':[True]*4,'top':[True,False,True,False],'bottom':[False,True,False,True],'left':[True,True,False,False],'right':[False,False,True,True],'pcm-top':[False,True,False,True],'pcm-bottom':[True,False,False,True]}
    for topology,fields in [('frame',[False,False]),('field',[True,True]),('mixed',[False,True]),('reverse-mixed',[True,False])]:
        for pattern,switching in patterns.items():
            pcm={0} if pattern=="pcm-top" else ({1} if pattern=="pcm-bottom" else set())
            for constrained in [False,True]:
                for coded in [False,True]:
                    for split,reverse in [(False,False),(True,False),(True,True)]:
                        config=configuration(constrained);groups=[[0,1],[2,3]] if split else [[0,1,2,3]]
                        frames=[];packets=[];gold=bytearray()
                        for frame,qs in enumerate([0,26,51]):
                            nals=[slice_nal(group,fields,switching,frame,qs,coded,pcm) for group in (groups[::-1] if reverse else groups)]
                            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                            frames.append((frame,frame==0,packet));packets.append(packet.hex());gold.extend(reconstruct(fields,switching,qs,coded,constrained,groups,pcm))
                        name=f'avc-si-mbaff-{topology}-{pattern}-constrained{int(constrained)}-'+('signed' if coded else 'zero')+('-split' if split else '')+('-aso' if reverse else '')
                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,frame_picture=True))
    (DEST/'avc-si-mbaff.json').write_text(json.dumps(dict(cases=cases,provenance='Original Extended-profile MBAFF SI and ordinary intra4/PCM macroblocks. Independent per-sample and four-sample chroma DC availability labels and spatial DC/matrix switching oracle, QS0/26/51, signed luma residual, frame/field/mixed pairs, constrained intra, one/two slices and reversed arrival. Filtering disabled. SI4 mode flags follow H.264 table7-12 and7.3.5.1; JM19 omits those flags and is not a SI syntax oracle. No private media, FFmpeg, external encoder or network.'),indent=2)+'\n')

if __name__=='__main__':main()
