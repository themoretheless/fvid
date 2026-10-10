#!/usr/bin/env python3
"""Original mixed MBAFF SI/P/SP/B pictures and retained zero-motion P frames."""
import argparse
import json
from generate_avc_switching_mbaff_fixtures import Writer, DEST, indices, slice_nal as sp_slice, reconstruct as sp_reconstruct
from generate_avc_si_mbaff_fixtures import configuration, slice_nal as si_slice, reconstruct as si_reconstruct
from generate_avc_mbaff_switching_filter_fixtures import filter_picture
from generate_avc_mbaff_si_filter_fixtures import LAYOUTS
from avc_fixture_mp4 import mux


def inter_slice(pair,field,frame,kind,mode):
    """P or B-L0 16x16, zero motion/residual; field ref index0."""
    b=Writer();b.ue(pair);b.ue(1 if kind=='b' else 0);b.ue(0);b.u(frame,4);b.u(0);b.u(frame*2,4)
    if kind=='b':b.u(1) # spatial-direct flag (explicit L0 MBs below)
    b.u(0);b.u(0) # default reference counts, L0 modifications absent
    if kind=='b':b.u(0) # L1 modifications absent
    b.u(0);b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    for parity in range(2):
        b.ue(0)
        if parity==0:b.u(int(field))
        b.ue(1 if kind=='b' else 0)
        if field:b.u(1)
        b.se(0);b.se(0);b.ue(0)
    return b.nal(0x41)


def make_case(topology,fields,kind,left,inside,constrained,coded,aso,mode):
    si_pairs={0,2} if left else {1,3}
    switching=[True if inside=='all' else address%2==0 for address in range(8)]
    groups=[[2*p,2*p+1] for p in sorted(si_pairs)]
    config=configuration(constrained,64)
    previous=[[base+sign*(x//4*2+y//4)+(y%2) for y in range(h) for x in range(w)]
              for base,sign,w,h in [(120,1,32,64),(124,1,16,32),(132,-1,16,32)]]
    frames=[];packets=[];gold=bytearray()
    for frame,qs in enumerate([26,0,26,51,26]):
        nals=[]
        for pair in (range(3,-1,-1) if aso else range(4)):
            if frame==0:nals.append(sp_slice(pair,fields[pair],0,'primary',26,False,previous,'none'))
            elif frame==4:nals.append(inter_slice(pair,fields[pair],frame,'p',1))
            elif pair in si_pairs:nals.append(si_slice([2*pair,2*pair+1],fields,switching,frame,qs,coded,set(),'both' if coded else None,64,mode))
            elif kind in ['primary','secondary']:nals.append(sp_slice(pair,fields[pair],frame,kind,qs,coded,previous,'none','both' if coded else None,dc_level=64,mode=mode))
            else:nals.append(inter_slice(pair,fields[pair],frame,kind,mode))
        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
        frames.append((frame,frame==0,packet));packets.append(packet.hex())
        if frame not in [0,4]:
            unfiltered=sp_reconstruct(previous,fields,kind,qs,coded,'none','both' if coded else None,dc_level=64) if kind in ['primary','secondary'] else [p[:] for p in previous]
            intra=si_reconstruct(fields,switching,qs,coded,constrained,groups,set(),'both' if coded else None,64)
            intra=[intra[:2048],intra[2048:2560],intra[2560:]]
            for pair in si_pairs:
                for parity in range(2):
                    for component in range(3):
                        for pos in indices(pair,parity,fields[pair],component):unfiltered[component][pos]=intra[component][pos]
            switching_blocks=set(range(8)) if kind in ['primary','secondary'] else {2*p+parity for p in si_pairs for parity in range(2)}
            previous=filter_picture(unfiltered,fields,mode,switching_blocks=switching_blocks)
        gold.extend(bytes(v for plane in previous for v in plane))
    assert gold[-3072:]==gold[-6144:-3072] # retained P is an exact reference copy
    name=f'avc-mbaff-mixed-si-inter-{topology}-{kind}-si'+('left' if left else 'right')+f'-{inside}-c{int(constrained)}-'+('signed' if coded else 'zero')+('-aso' if aso else '')+f'-mode{mode}'
    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,64,60))
    (DEST/(name+'-reference.yuv')).write_bytes(gold)
    return dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=5,frame_picture=True,width=32,height=64)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--reproducer-only',action='store_true');args=parser.parse_args()
    cases=[]
    layouts={'frame':LAYOUTS['frame']} if args.reproducer_only else LAYOUTS
    for topology,fields in layouts.items():
        for kind in (['p'] if args.reproducer_only else ['p','primary','secondary','b']):
            for left in [True,False]:
                for inside in (['all'] if args.reproducer_only else ['all','top']):
                    for constrained in ([False] if args.reproducer_only else [False,True]):
                        for coded in ([False] if args.reproducer_only else [False,True]):
                            for aso in ([False] if args.reproducer_only else [False,True]):
                                for mode in ([1] if args.reproducer_only else [0,1,2]):cases.append(make_case(topology,fields,kind,left,inside,constrained,coded,aso,mode))
    if not args.reproducer_only:
        observable={kind:dict(internal=0,external=0) for kind in ['p','primary','secondary','b']}
        aso_equal=0
        for case in cases:
            name=case['reference'];data=(DEST/name).read_bytes()
            if '-aso-mode' in name:
                assert data==(DEST/name.replace('-aso-mode','-mode')).read_bytes()
                aso_equal+=1
            if '-mode0-' not in name:continue
            kind=next(k for k in observable if '-'+k+'-si' in name)
            disabled=(DEST/name.replace('-mode0-','-mode1-')).read_bytes()
            internal=(DEST/name.replace('-mode0-','-mode2-')).read_bytes()
            observable[kind]['internal']+=internal!=disabled
            observable[kind]['external']+=data!=internal
        assert all(all(v.values()) for v in observable.values()),observable
        print('Observable filter controls:',observable,'ASO equal:',aso_equal)
    manifest='avc-mbaff-mixed-si-inter-reproducer.json' if args.reproducer_only else 'avc-mbaff-mixed-si-inter.json'
    (DEST/manifest).write_text(json.dumps(dict(cases=cases,provenance='Original32x64 PCM then three mixed SI/P/SP/B frames at QS0/26/51 and retained P frame. Six two-row layouts, SI-left/right pair slices, SI4/ordinary I4 pattern, constrained intra, zero or signed luma and escaped chroma DC64/AC, ASO, filter0/1/2. Own spatial/switching and physical deblocking oracle; uncoded P/B use zero-motion L0 reference and ordinary boundary strengths. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
    print(manifest,len(cases))

if __name__=='__main__':main()
