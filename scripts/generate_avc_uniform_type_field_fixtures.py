#!/usr/bin/env python3
"""Synthetic slice_type5..9 promise controls, independent of test execution."""
import json
from generate_avc_switching_field_fixtures import DEST,configuration,pcm,switching,reconstruct,weave,mux
from generate_avc_switching_field_filter_fixtures import smooth


def main():
    config=configuration(8,max_refs=3);cases=[];invalid=[]
    for kind in ['plain','primary','secondary']:
        for uniform_mb in [0,1]:
            for aso in [False,True]:
                fields={b:smooth(b) for b in [False,True]};frames=[];packets=[]
                def append(nals,idr=False):
                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                    frames.append((len(frames),idr,packet));packets.append(packet.hex())
                for i,bottom in enumerate([False,True]):append([pcm(bottom,i==0,False,fields[bottom])],i==0)
                raw=bytearray(weave(fields))
                for bottom in [False,True]:
                    ns=[switching(bottom,1,26,kind,False,False,False,addresses=[mb],uniform=mb==uniform_mb) for mb in range(2)]
                    append(ns[::-1] if aso else ns)
                    if kind!='plain':fields[bottom]=reconstruct(fields[bottom],26,kind,False)
                raw.extend(weave(fields))
                name=f'avc-uniform-type-field-{kind}-mb{uniform_mb}'+('-aso' if aso else '')
                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                (DEST/(name+'-reference.yuv')).write_bytes(raw)
                cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=2))
    for uniform_kind in ['plain','primary']:
        for uniform_mb in [0,1]:
            for aso in [False,True]:
                frames=[];packets=[]
                def append(nals,idr=False):
                    packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
                    frames.append((len(frames),idr,packet));packets.append(packet.hex())
                for i,bottom in enumerate([False,True]):append([pcm(bottom,i==0,False,smooth(bottom))],i==0)
                for bottom in [False,True]:
                    ns=[switching(bottom,1,26,uniform_kind if mb==uniform_mb else ('primary' if uniform_kind=='plain' else 'plain'),False,False,False,addresses=[mb],uniform=mb==uniform_mb) for mb in range(2)]
                    append(ns[::-1] if aso else ns)
                name=f'avc-invalid-uniform-type-field-{uniform_kind}-mb{uniform_mb}'+('-aso' if aso else '')
                (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60))
                invalid.append(dict(file=name+'-synthetic.mp4',configuration=config.hex(),packets=packets))
    (DEST/'avc-uniform-type-fields.avcc').write_bytes(config)
    for i in range(2):
        (DEST/f'avc-uniform-type-fields-pcm{i}.bin').write_bytes(bytes.fromhex(cases[0]['packets'][i]))
    for case in invalid:
        (DEST/case['file'].replace('-synthetic.mp4','-packet.bin')).write_bytes(bytes.fromhex(case['packets'][2]))
    (DEST/'avc-uniform-type-fields.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,
        provenance='Original PCM field pair followed by uniform-type P or SP fields with one flag5/8 and matching type0/3, or intentionally conflicting P/SP type promises. Both flag positions and NAL orders; no private media, external codec, FFmpeg or network.'),indent=2)+'\n')
    print('Uniform type controls:',len(cases),'valid,',len(invalid),'invalid')


if __name__=='__main__':main()
