#!/usr/bin/env python3
"""Original MBAFF primary/secondary SP signed filter offsets and reference history."""
import json
from generate_avc_switching_mbaff_fixtures import DEST, configuration, slice_nal, reconstruct
from generate_avc_mbaff_switching_filter_fixtures import filter_picture
from generate_avc_mbaff_si_filter_fixtures import LAYOUTS
from avc_fixture_mp4 import mux


def main():
    config=configuration(64);cases=[];controls={}
    for topology,fields in LAYOUTS.items():
        for kind in ['primary','secondary']:
            for coded in [False,True]:
                for reverse in [False,True]:
                    for offsets in [(-4,-2),(4,2)]:
                        for mode in [0,1,2]:
                            previous=[[base+sign*(x//4*3+y//4*2)+(y%2)*2 for y in range(h) for x in range(w)]
                                      for base,sign,w,h in [(64,1,32,64),(96,1,16,32),(160,-1,16,32)]]
                            frames=[];packets=[];gold=bytearray()
                            for frame,qs in enumerate([26,0,26,51]):
                                nals=[slice_nal(pair,fields[pair],frame,kind,qs,coded,previous,'none',
                                                mode=mode if frame else 1,offsets=offsets)
                                      for pair in (range(3,-1,-1) if reverse else range(4))]
                                packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                frames.append((frame,frame==0,packet));packets.append(packet.hex())
                                if frame:previous=filter_picture(reconstruct(previous,fields,kind,qs,coded,'none'),fields,mode,offsets)
                                gold.extend(bytes(v for plane in previous for v in plane))
                            stem=f'{topology}-{kind}-'+('signed' if coded else 'zero')+('-aso' if reverse else '')
                            baseline=(DEST/(f'avc-mbaff-switching-filter-{stem}-mode{mode}-reference.yuv')).read_bytes()
                            controls[(topology,kind,coded,reverse,offsets,mode)]=(bytes(gold),baseline)
                            name=f'avc-mbaff-sp-filter-offsets-{stem}-a{offsets[0]}-b{offsets[1]}-mode{mode}'
                            (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,64,60))
                            (DEST/(name+'-reference.yuv')).write_bytes(gold)
                            cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=4,frame_picture=True,width=32,height=64))
    observable={kind:0 for kind in ['primary','secondary']}
    for (_,kind,_,_,_,mode),(data,baseline) in controls.items():
        if mode==1:assert data==baseline
        else:observable[kind]+=data!=baseline
    assert all(observable.values()),observable
    (DEST/'avc-mbaff-sp-filter-offsets.json').write_text(json.dumps(dict(cases=cases,observable=observable,
        provenance='Original32x64 PCM then primary/secondary SP QS0/26/51, six two-row layouts, zero/signed residual, pair slices/ASO, filter modes0/1/2 and actual offsets(-4,-2)/(4,2). Own scalar switching and physical H.2648.7 filter oracle, filtered reference history. Mode1 controls equal existing zero-offset fixtures. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),observable)

if __name__=='__main__':main()
