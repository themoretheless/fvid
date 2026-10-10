#!/usr/bin/env python3
"""Original raster SI/P/SP/B mixtures, two PCM references and independent YUV."""
import argparse
import json
from generate_avc_si_mbaff_fixtures import DEST, Writer, positions, configuration, slice_nal as si_slice, reconstruct as si_reconstruct
from generate_avc_switching_mbaff_fixtures import write_chroma_dc_one, reconstruct as sp_reconstruct
from generate_avc_switching_field_filter_fixtures import filter_plane
from avc_fixture_mp4 import mux

KINDS=['p-coded','p-skip','primary','primary-skip','secondary','secondary-skip',
       'b-l0','b-l1','b-bi','b-spatial','b-temporal','b-skip']
GROUPS={'top':[[0,1],[2,3]],'bottom':[[0,1],[2,3]],'left':[[0],[1],[2],[3]],'right':[[0],[1],[2],[3]]}
SI={'top':{0,1},'bottom':{2,3},'left':{0,2},'right':{1,3}}


def header(address,frame,kind,qs,mode,interlaced):
    b=Writer();b.ue(address)
    code=2 if kind=='pcm' else (1 if kind.startswith('b-') else (3 if kind.startswith(('primary','secondary')) else 0))
    b.ue(code);b.ue(0);b.u(frame,4)
    if interlaced:b.u(0)
    if frame==0:b.ue(0)
    b.u(frame*2,4)
    if code==1:b.u(int(kind!='b-temporal'))
    if code!=2:
        b.u(int(code==1))
        if code==1:b.ue(min(frame,4)-1);b.ue(0)
        b.u(0)
        if code==1:b.u(0)
    if frame==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0)
    if code==3:b.u(int(kind.startswith('secondary')));b.se(qs-26)
    b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    return b


def inter_slice(addresses,frame,kind,qs,coded,mode,interlaced,source=None):
    b=header(addresses[0],frame,kind,qs,mode,interlaced)
    if kind.endswith('skip'):b.ue(len(addresses));return b.nal(0x41)
    for address in addresses:
        if kind=='pcm':
            b.ue(25);b.align()
            for component in range(3):
                for pos in positions(address//2,address%2,False,component,True):b.u(source[component][pos],8)
            continue
        b.ue(0) # mb_skip_run
        direct=kind in ['b-spatial','b-temporal']
        b.ue(0 if direct else {'b-l0':1,'b-l1':2,'b-bi':3}.get(kind,0))
        if kind.startswith('b-'):
            if not direct:
                lists=[0] if kind=='b-l0' else ([1] if kind=='b-l1' else [0,1])
                for prediction_list in lists:
                    if prediction_list==0 and min(frame,4)>1:
                        if min(frame,4)==2:b.u(1)
                        else:b.ue(0)
                for _ in lists:b.se(0);b.se(0)
        else:b.se(0);b.se(0)
        active=coded and kind in ['primary','secondary']
        b.ue(12 if active else 0)
        if active:
            b.se(0);pattern=2*(address%2)+address//2
            for block in range(16):b.u(1,2);b.u((block+pattern)%2);b.u(1)
            for component in range(2):write_chroma_dc_one(b,(component+pattern)%2,64)
            for component in range(2):
                for block in range(4):b.u(1,2);b.u((block+component+pattern)%2);b.u(1)
    return b.nal(0x65 if frame==0 else 0x41)


def make_case(sequence,layout,kind,constrained,coded,aso,mode):
    interlaced=sequence=='paff';groups=GROUPS[layout];si_addresses=SI[layout]
    si_groups=[group for group in groups if group[0] in si_addresses]
    switching=[True]*4
    for group in si_groups:
        if len(group)>1:switching[group[1]]=False # constrained ordinary I4 neighbour
    config=configuration(constrained,32,True,interlaced,4)
    history={};motion={};frames=[];packets=[];gold=bytearray()
    identities={address:index for index,group in enumerate(groups) for address in group}
    for frame,qs in enumerate([26,26,0,26,51,26]):
        if frame<2:
            current=[[base+sign*(x//4*2+y//4)+frame*7 for y in range(h) for x in range(w)]
                     for base,sign,w,h in [(112,1,32,32),(124,1,16,16),(140,-1,16,16)]]
            nals=[inter_slice(list(range(4)),frame,'pcm',26,False,1,interlaced,current)]
            metadata={address:None for address in range(4)}
        elif frame==5:
            current=[plane[:] for plane in history[frame-1]]
            nals=[inter_slice(list(range(4)),frame,'p-coded',26,False,1,interlaced)]
            metadata={address:(frame-1,None) for address in range(4)}
        else:
            nals=[si_slice(group,[False,False],switching,frame,qs,coded,set(),'both' if coded else None,64,mode,
                           raster=True,interlaced=interlaced)
                  if group[0] in si_addresses else inter_slice(group,frame,kind,qs,coded,mode,interlaced)
                  for group in (groups[::-1] if aso else groups)]
            previous=history[frame-1]
            if kind.startswith(('primary','secondary')):
                current=sp_reconstruct(previous,[False,False],kind.split('-')[0],qs,coded and not kind.endswith('skip'),'none',
                                       'both' if coded and not kind.endswith('skip') else None,dc_level=64)
            else:current=[plane[:] for plane in previous]
            metadata={}
            latest=frame-1;list1=frame-2 # identical B lists swap their first two IDs
            for address in range(4):
                if address in si_addresses:metadata[address]=None;continue
                if kind=='b-l1':refs=(None,list1)
                elif kind in ['b-bi','b-spatial','b-skip']:refs=(latest,list1)
                elif kind=='b-temporal':
                    colocated=motion[list1][address]
                    ref0=latest if colocated is None else next(ref for ref in colocated if ref is not None)
                    assert ref0 in range(max(0,frame-4),frame)
                    refs=(ref0,list1)
                else:refs=(latest,None)
                metadata[address]=refs
                if not kind.startswith(('primary','secondary')):
                    for component in range(3):
                        for pos in positions(address//2,address%2,False,component,True):
                            values=[history[ref][component][pos] for ref in refs if ref is not None]
                            current[component][pos]=(sum(values)+len(values)//2)//len(values)
            intra=si_reconstruct([False,False],switching,qs,coded,constrained,si_groups,set(),'both' if coded else None,64,True)
            intra=[intra[:1024],intra[1024:1280],intra[1280:]]
            for address in si_addresses:
                for component in range(3):
                    for pos in positions(address//2,address%2,False,component,True):current[component][pos]=intra[component][pos]
            intra_strength=set(range(4)) if kind.startswith(('primary','secondary')) else si_addresses
            current=[filter_plane(plane,w,h,c>0,mode,len(groups),owners=identities,horizontal_strength=4,
                                  switching_blocks=intra_strength) for c,(plane,w,h) in enumerate(zip(current,[32,16,16],[32,16,16]))]
        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
        frames.append((frame,frame==0,packet));packets.append(packet.hex());history[frame]=current;motion[frame]=metadata
        gold.extend(bytes(v for plane in current for v in plane))
    assert gold[-1536:]==gold[-3072:-1536]
    name=f'avc-raster-mixed-si-inter-{sequence}-{layout}-{kind}-c{int(constrained)}-'+('signed' if coded else 'zero')+('-aso' if aso else '')+f'-mode{mode}'
    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
    return dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,
                frame_count=6,width=32,height=32,sequence=sequence,layout=layout,kind=kind,constrained=constrained,coded=coded,aso=aso,mode=mode)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--reproducer-only',action='store_true');args=parser.parse_args();cases=[]
    for sequence in (['progressive'] if args.reproducer_only else ['progressive','paff']):
        for layout in (['top','bottom'] if args.reproducer_only else GROUPS):
            for kind in (['p-coded'] if args.reproducer_only else KINDS):
                for constrained in ([False] if args.reproducer_only else [False,True]):
                    for coded in ([False] if args.reproducer_only else [False,True]):
                        for aso in ([False] if args.reproducer_only else [False,True]):
                            for mode in ([1] if args.reproducer_only else [0,1,2]):cases.append(make_case(sequence,layout,kind,constrained,coded,aso,mode))
    if not args.reproducer_only:
        keys=['sequence','layout','kind','constrained','coded','aso','mode']
        table={tuple(c[k] for k in keys):c for c in cases}
        def pixels(case):return (DEST/case['reference']).read_bytes()
        def peer(case,**changes):return table[tuple(changes.get(k,case[k]) for k in keys)]
        observable={kind:dict(internal=0,external=0) for kind in KINDS}
        aso_equal=0;paff_equal=0;constrained_changed=0;temporal_changed=0;l1_changed=0
        for case in cases:
            data=pixels(case)
            if case['aso']:
                assert data==pixels(peer(case,aso=False));aso_equal+=1
            if case['sequence']=='paff':
                assert data==pixels(peer(case,sequence='progressive'));paff_equal+=1
            if not case['constrained']:constrained_changed+=data!=pixels(peer(case,constrained=True))
            if case['kind']=='b-temporal' and case['mode']==1:temporal_changed+=data!=pixels(peer(case,kind='b-spatial'))
            if case['kind']=='b-l1' and case['mode']==1:l1_changed+=data!=pixels(peer(case,kind='b-l0'))
            if case['kind'] in ['p-skip','b-skip']:
                assert data==pixels(peer(case,kind='p-coded' if case['kind']=='p-skip' else 'b-spatial'))
            if case['mode']==0:
                internal=pixels(peer(case,mode=2));disabled=pixels(peer(case,mode=1))
                observable[case['kind']]['internal']+=internal!=disabled
                observable[case['kind']]['external']+=data!=internal
        assert all(all(v.values()) for v in observable.values()),observable
        assert constrained_changed and temporal_changed and l1_changed
        print('Controls:',observable,'ASO:',aso_equal,'PAFF:',paff_equal,
              'constrained:',constrained_changed,'temporal:',temporal_changed,'L1:',l1_changed)
    manifest='avc-raster-mixed-si-inter-reproducer.json' if args.reproducer_only else 'avc-raster-mixed-si-inter.json'
    (DEST/manifest).write_text(json.dumps(dict(cases=cases,provenance='Original32x32 progressive/PAFF full frames: two distinct PCM references, three SI/P/SP/B pictures QS0/26/51, retained P. Row/column SI positions, constrained SI/I4, signed luma and escaped chroma DC64/AC, ASO, filter0/1/2. P/SP skip/explicit and B L0/L1/Bi/spatial/temporal direct/skip; B lists and temporal co-located reference IDs tracked independently. Own scalar spatial/switching, average prediction and H.2648.7 filter oracle. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
    print(manifest,len(cases))

if __name__=='__main__':main()
