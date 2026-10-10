#!/usr/bin/env python3
"""Original mixed SI/SP with explicit/direct/skip B fields and scalar oracle."""
import argparse
import json
from generate_avc_switching_field_fixtures import configuration, Writer, pcm, switching, reconstruct, weave, mux, DEST
from generate_avc_mixed_si_sp_field_fixtures import backdrop, replace_mb
from generate_avc_switching_field_filter_fixtures import filter_plane
from generate_avc_switching_field_motion_fixtures import compensate


def b_slice(bottom,reverse,mb,mode,kind,field_index=0):
    b=Writer();b.ue(mb);b.ue(1);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom))
    b.u(2+int(bottom != reverse),4);b.u(int(not kind.startswith('temporal')))
    if kind.startswith('temporal'):
        b.u(1);b.ue(1+field_index);b.ue(0) # retain both/three L0 field identities
    else:b.u(0) # default one reference per list
    b.u(0);b.u(0) # reference list modifications
    b.u(0) # adaptive marking
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if kind.endswith('skip'):b.ue(1)
    elif kind in ['spatial','temporal']:b.ue(0);b.ue(0);b.ue(0)
    else:
        b.ue(0);b.ue(dict(l0=1,l1=2,bi=3)[kind])
        for _ in range(2 if kind=='bi' else 1):b.se(0);b.se(0)
        b.ue(0) # explicit B 16x16, zero motion, no residual
    return b.nal(0x41)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--direct-only',action='store_true');args=parser.parse_args()
    kinds=['spatial','spatial-skip','temporal','temporal-skip'] if args.direct_only else ['l0','l1','bi']
    prefix='avc-mixed-switching-b-direct-field-' if args.direct_only else 'avc-mixed-si-b-field-'
    cases=[];config=configuration(8,max_refs=3)
    for switching_kind in ['si','primary','secondary']:
        for kind in kinds:
            for reverse in [False,True]:
                for b_mb in [0,1]:
                    for aso in [False,True]:
                        for coded in [False,True]:
                            for qs in [0,26,51]:
                                for mode in [0,1,2]:
                                    fields={b:backdrop(b) for b in [False,True]};initial={b:[p[:] for p in fields[b]] for b in fields};order=[True,False] if reverse else [False,True]
                                    frames=[];packets=[];gold=bytearray()
                                    def append(nals,idr=False):
                                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                        packets.append(packet.hex());frames.append((len(frames),idr,packet))
                                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                    gold.extend(weave(fields))
                                    for field_index,bottom in enumerate(order):
                                        nals=[b_slice(bottom,reverse,mb,mode,kind,field_index) if mb==b_mb else switching(bottom,1,qs,switching_kind,coded,False,reverse,deblock=mode,addresses=[mb]) for mb in range(2)]
                                        append(nals[::-1] if aso else nals)
                                        opposite=[compensate(p,w,h,(0,2 if bottom else -2),True) if c else p[:] for c,(p,w,h) in enumerate(zip(fields[not bottom],[32,16,16],[16,8,8]))]
                                        l0=fields[bottom]
                                        if kind.startswith('temporal') and field_index:
                                            l0=[compensate(p,w,h,(0,2 if bottom else -2),True) if c else p[:] for c,(p,w,h) in enumerate(zip(initial[not bottom],[32,16,16],[16,8,8]))]
                                        current=[p[:] for p in l0] if kind=='l0' else (opposite if kind=='l1' else [[(a+b+1)//2 for a,b in zip(p,q)] for p,q in zip(l0,opposite)])
                                        replace_mb(current,reconstruct(fields[bottom],qs,switching_kind,coded,independent_slices=True),1-b_mb)
                                        fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},switching_blocks={1-b_mb}) for c,(p,w,h) in enumerate(zip(current,[32,16,16],[16,8,8]))]
                                    gold.extend(weave(fields))
                                    for bottom in order:append([switching(bottom,2,26,'plain',False,False,reverse)])
                                    gold.extend(weave(fields))
                                    variant='' if switching_kind=='si' else switching_kind+'-'
                                    name=f'{prefix}{variant}{kind}-r{int(reverse)}-b{b_mb}-a{int(aso)}-c{int(coded)}-qs{qs}-mode{mode}'
                                    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
                                    cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,kind=kind,switching_kind=switching_kind))
    if not args.direct_only:
        # Read-only controls prove this is not a degenerate reference/filter matrix.
        filtered={kind:0 for kind in ['l0','l1','bi']};different={kind:0 for kind in ['l1','bi']}
        for case in cases:
            name=case['reference'];data=(DEST/name).read_bytes()
            if '-a1-' in name:assert data==(DEST/name.replace('-a1-','-a0-')).read_bytes()
            if '-mode0-' in name:filtered[case['kind']]+=data!=(DEST/name.replace('-mode0-','-mode2-')).read_bytes()
            if case['kind']!='l0':different[case['kind']]+=data!=(DEST/name.replace('-'+case['kind']+'-','-l0-')).read_bytes()
        assert all(filtered.values()) and all(different.values()),(filtered,different)
        print('Observable external filtering:',filtered,'distinct B prediction:',different)
    else:
        filtered={kind:0 for kind in kinds};temporal_changed=0
        for case in cases:
            name=case['reference'];data=(DEST/name).read_bytes();kind=case['kind']
            if '-a1-' in name:assert data==(DEST/name.replace('-a1-','-a0-')).read_bytes()
            if kind.endswith('-skip'):assert data==(DEST/name.replace('-'+kind+'-','-'+kind[:-5]+'-')).read_bytes()
            if kind=='spatial':
                peer=name.replace(prefix,'avc-mixed-si-b-field-').replace('-spatial-','-bi-')
                assert data==(DEST/peer).read_bytes()
            if kind=='temporal':temporal_changed+=data!=(DEST/name.replace('-temporal-','-spatial-')).read_bytes()
            if '-mode0-' in name:filtered[kind]+=data!=(DEST/name.replace('-mode0-','-mode2-')).read_bytes()
        assert all(filtered.values()) and temporal_changed,(filtered,temporal_changed)
        print('Direct controls:',filtered,'temporal differs:',temporal_changed)
    manifest='avc-mixed-switching-b-direct-fields.json' if args.direct_only else 'avc-mixed-si-b-fields.json'
    (DEST/manifest).write_text(json.dumps(dict(cases=cases,provenance=('Original PCM pair, mixed SI/primary-SP/secondary-SP beside spatial/temporal B-direct/skip, retained P pair. Both parity orders and switching positions, ASO, QS0/26/51, zero/signed switching residual, filter0/1/2. Scalar field-list identity oracle, opposite-parity chroma interpolation, switching and deblocking; temporal second field maps co-located original opposite PCM into L0 index2. No private media, external codec, FFmpeg or network.' if args.direct_only else 'Original PCM pair, mixed SI/primary-SP/secondary-SP with B-L0/L1/Bi zero-motion pair, retained P pair; both parity orders and SI positions, ASO, QS0/26/51, signed SI residuals, filter0/1/2. Own scalar spatial/switching/filter oracle; no private media, external codec, FFmpeg or network.')),indent=2)+'\n')
    print('Mixed SI/B fields:',len(cases))

if __name__=='__main__':main()
