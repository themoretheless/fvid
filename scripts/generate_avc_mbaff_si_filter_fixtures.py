#!/usr/bin/env python3
"""Original two-row MBAFF SI spatial prediction and signed deblocking offsets."""
import json
from generate_avc_si_mbaff_fixtures import DEST, configuration, slice_nal, reconstruct
from generate_avc_mbaff_switching_filter_fixtures import filter_picture
from avc_fixture_mp4 import mux

LAYOUTS={'frame':[False]*4,'field':[True]*4,'checker':[False,True,True,False],
         'reverse-checker':[True,False,False,True],'field-above-frame':[True,True,False,False],
         'frame-above-field':[False,False,True,True]}
OFFSETS=[(-4,-2),(0,0),(4,2)]


def main():
    cases=[];controls={};constrained_controls={}
    patterns={'all':[True]*8,'top':[True,False]*4,'bottom':[False,True]*4}
    for topology,fields in LAYOUTS.items():
        for pattern,switching in patterns.items():
            for constrained in [False,True]:
                config=configuration(constrained,64)
                for coded in [False,True]:
                    for grouping in ['whole','pairs','aso']:
                        groups=[list(range(8))] if grouping=='whole' else [[2*p,2*p+1] for p in range(4)]
                        identities=[None]*8
                        for slice_id,group in enumerate(groups):
                            for address in group:identities[address]=slice_id
                        for offsets in OFFSETS:
                            for mode in [0,1,2]:
                                frames=[];packets=[];gold=bytearray()
                                for frame,qs in enumerate([0,26,51]):
                                    nals=[slice_nal(group,fields,switching,frame,qs,coded,set(),
                                                    'both' if coded else None,64,mode,offsets)
                                          for group in (groups[::-1] if grouping=='aso' else groups)]
                                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                    frames.append((frame,frame==0,packet));packets.append(packet.hex())
                                    raw=reconstruct(fields,switching,qs,coded,constrained,groups,set(),'both' if coded else None,64)
                                    planes=[list(raw[:2048]),list(raw[2048:2560]),list(raw[2560:])]
                                    filtered=filter_picture(planes,fields,mode,offsets,identities)
                                    gold.extend(bytes(v for plane in filtered for v in plane))
                                key=(topology,pattern,constrained,coded,grouping,offsets)
                                controls.setdefault(key,{})[mode]=bytes(gold)
                                constrained_controls.setdefault((topology,pattern,coded,grouping,offsets,mode),{})[constrained]=bytes(gold)
                                name=f'avc-mbaff-si-filter-{topology}-{pattern}-c{int(constrained)}-'+('signed' if coded else 'zero')+f'-{grouping}-a{offsets[0]}-b{offsets[1]}-mode{mode}'
                                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,64,60))
                                (DEST/(name+'-reference.yuv')).write_bytes(gold)
                                cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,frame_picture=True,width=32,height=64))
    internal=sum(data[2]!=data[1] for data in controls.values())
    external=sum(data[0]!=data[2] for key,data in controls.items() if key[4]!='whole')
    constrained=sum(data[False]!=data[True] for data in constrained_controls.values())
    offsets_changed=0
    for key,data in controls.items():
        if key[-1]!=(0,0):offsets_changed+=data[0]!=controls[key[:-1]+((0,0),)][0]
    assert internal and external and constrained and offsets_changed,(internal,external,constrained,offsets_changed)
    (DEST/'avc-mbaff-si-filter.json').write_text(json.dumps(dict(cases=cases,
        observable=dict(internal=internal,external=external,constrained=constrained,offsets=offsets_changed),
        provenance='Original32x64 SI/ordinary intra4 DC prediction, six two-row layouts, QS0/26/51, zero or signed luma plus escaped chroma DC64 and AC, constrained intra, whole/pair/ASO slices, filter modes0/1/2 and actual offsets(-4,-2)/(0,0)/(4,2). Independent per-sample spatial availability then physical ownership/scalar H.2648.7 filter oracle at QP26. No private media, external codec, FFmpeg or network. SI4 flags follow specification; JM19 is not a SI syntax oracle.'),indent=2)+'\n')
    print(len(cases),dict(internal=internal,external=external,constrained=constrained,offsets=offsets_changed))

if __name__=='__main__':main()
