#!/usr/bin/env python3
"""Original mixed SI/SP complementary fields with independent scalar oracle."""
import json
from generate_avc_switching_field_fixtures import DEST, configuration, pcm, switching, reconstruct, weave, mux
from generate_avc_switching_field_filter_fixtures import smooth, filter_plane


def replace_mb(destination, source, mb):
    for target,previous,width,size in zip(destination,source,[32,16,16],[16,8,8]):
        for row in range(size):
            start=row*width+mb*size
            target[start:start+size]=previous[start:start+size]


def backdrop(bottom):
    # Keep the reference near SI DC128 to exercise external filtering.
    return [[v+shift for v in plane]
            for plane,shift in zip(smooth(bottom),[56,24,-24])]


def main():
    config=configuration(8,max_refs=3);cases=[]
    for intra_kind in ['si']:
        for kind in ['primary','secondary']:
            for coded in [False,True]:
                for reverse in [False,True]:
                    for sp_mb in [0,1]:
                        for aso in [False,True]:
                            for qs in [0,26,51]:
                                for mode in [0,1,2]:
                                    fields={b:backdrop(b) for b in [False,True]};frames=[];packets=[]
                                    order=[True,False] if reverse else [False,True]
                                    def append(nals,idr=False):
                                        packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                                        frames.append((len(frames),idr,packet));packets.append(packet.hex())
                                    for i,bottom in enumerate(order):append([pcm(bottom,i==0,reverse,fields[bottom])],i==0)
                                    raw=bytearray(weave(fields))
                                    for bottom in order:
                                        intra_source=reconstruct(fields[bottom],qs,'si',coded,independent_slices=True)
                                        nals=[switching(bottom,1,qs,kind,coded,False,reverse,deblock=mode,addresses=[mb]) if mb==sp_mb
                                              else switching(bottom,1,qs,'si',coded,False,reverse,deblock=mode,addresses=[mb]) for mb in range(2)]
                                        append(nals[::-1] if aso else nals)
                                        unfiltered=reconstruct(fields[bottom],qs,kind,coded)
                                        replace_mb(unfiltered,intra_source,1-sp_mb)
                                        fields[bottom]=[filter_plane(p,w,h,c>0,mode,2,owners={0:0,1:1},
                                                                    switching_blocks=None) for c,(p,w,h)
                                                        in enumerate(zip(unfiltered,[32,16,16],[16,8,8]))]
                                    raw.extend(weave(fields))
                                    # Subsequent whole P field validates retained mixed-picture references.
                                    for bottom in order:append([switching(bottom,2,26,'plain',False,False,reverse)])
                                    raw.extend(weave(fields))
                                    name=f'avc-mixed-si-sp-field-{intra_kind}-{kind}-'+('signed' if coded else 'zero')+'-'+('bottom-first' if reverse else 'top-first')+f'-sp{sp_mb}-qs{qs}-mode{mode}'+('-aso' if aso else '')
                                    (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                                    (DEST/(name+'-reference.yuv')).write_bytes(raw)
                                    cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',
                                        configuration=config.hex(),packets=packets,frame_count=3,kind=kind,coded=coded,
                                        reverse=reverse,sp_mb=sp_mb,aso=aso,qs=qs,mode=mode,intra_kind=intra_kind))
    (DEST/'avc-mixed-si-sp-fields.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original PCM pair, mixed SI/SP pair, retained P pair. Own scalar SI/SP and deblocking reference; independent slice spatial neighbours. No private media, FFmpeg or network.'),indent=2)+'\n')
    print('Mixed SI/SP field streams:',len(cases))


if __name__=='__main__':main()
