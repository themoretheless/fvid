#!/usr/bin/env python3
"""Original MBAFF SI signed DC/AC chroma, PCM contexts and constrained neighbors."""
import json
from generate_avc_si_mbaff_fixtures import DEST,configuration,slice_nal,reconstruct,mux


def main():
    cases=[]
    patterns={'all':[True]*4,'top':[True,False,True,False],'bottom':[False,True,False,True],'left':[True,True,False,False],'right':[False,False,True,True],'pcm-top':[False,True,False,True],'pcm-bottom':[True,False,False,True]}
    for topology,fields in [('frame',[False,False]),('field',[True,True]),('mixed',[False,True]),('reverse-mixed',[True,False])]:
        for pattern,switching in patterns.items():
            pcm={0} if pattern=='pcm-top' else ({1} if pattern=='pcm-bottom' else set())
            for constrained in [False,True]:
                for chroma_mode in ['dc','ac','both']:
                    for split,reverse in [(False,False),(True,False),(True,True)]:
                        config=configuration(constrained);groups=[[0,1],[2,3]] if split else [[0,1,2,3]]
                        frames=[];packets=[];gold=bytearray()
                        for frame,qs in enumerate([0,26,51]):
                            nals=[slice_nal(group,fields,switching,frame,qs,True,pcm,chroma_mode,64) for group in (groups[::-1] if reverse else groups)]
                            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals);frames.append((frame,frame==0,packet));packets.append(packet.hex())
                            gold.extend(reconstruct(fields,switching,qs,True,constrained,groups,pcm,chroma_mode,64))
                        name=f'avc-mbaff-si-chroma-{topology}-{pattern}-constrained{int(constrained)}-{chroma_mode}'+('-split' if split else '')+('-aso' if reverse else '')
                        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,32,32,60));(DEST/(name+'-reference.yuv')).write_bytes(gold)
                        cases.append(dict(file=name+'-synthetic.mp4',reference=name+'-reference.yuv',configuration=config.hex(),packets=packets,frame_count=3,frame_picture=True,dc_level=64))
    (DEST/'avc-mbaff-si-chroma.json').write_text(json.dumps(dict(cases=cases,provenance='Original MBAFF SI/ordinary intra4/PCM streams, frame/field/mixed geometry, QS0/26/51, signed luma and isolated DC/AC or combined chroma residual (escaped DC levels+/-64). Independent physical-sample spatial prediction, four-sample chroma boundary availability, DC transpose and frame/field AC scan. Own CAVLC context counts include PCM16; constrained intra, split slices and reversed arrival. Deblocking disabled. No private media, external codec, FFmpeg or network; JM19 is not a SI4 syntax oracle.'),indent=2)+'\n')

if __name__=='__main__':main()
