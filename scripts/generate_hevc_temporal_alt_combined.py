"""Owned spatial/Col/combined alternate-reference merge boundary movies."""
import json
import generate_hevc_temporal_alt_ref as a
import generate_hevc_depth_iv_b as b
from generate_hevc_cqt import reference,merge_index_bins,terminal_zero,OUT
from hevc_fixture_mp4 import mux

def target(size,index):
    encoder,contexts=b.banks(2);columns=size//32
    for y in range(columns):
        for x in range(columns):
            encoder.decision(contexts['split0'],False)
            encoder.decision(contexts[f'skip{int(x>0)+int(y>0)}'],True)
            # First CU establishes spatial motion using Col. Probe the next
            # CU, whose A1 has published alternate refs while Col retains 0.
            merge_index_bins(encoder,contexts,index if (x,y)==(1,0)else 0,5)
            if y*columns+x+1<columns**2:terminal_zero(encoder)
    return encoder.finish()

def generate():
    files={};cases=[]
    for size,direct in [(64,False),(128,False),(128,True)]:
        tex,_=reference(size,5,True,flat=True)
        past,_=reference(size,5,False);future,_=reference(size,5,False,bias=97)
        for index,refs,motion in [(0,[1,1],[2,-2]),(1,[1,1],[2,-2]),
                (2,[1,0],[2,-2]),(3,[0,1],[2,-2]),(4,[0,0],[0,0])]:
            units=[[b.header(layer,0,tex if layer==0 else past,2)for layer in [0,3,5,8]],
                [b.header(layer,2,tex if layer==0 else future,2,negative=[2])for layer in [0,3,5,8]],
                [b.header(0,1,tex,2,negative=[1],positive=[1]),
                 b.header(3,1,b.source_b(size,'bi'),0,negative=[1],positive=[1]),
                 b.header(5,1,past,2,negative=[1],positive=[1]),
                 a.target_header(target(size,index),True)]]
            stream=b''.join(b'\0\0\0\1'+n for n in b.parameters(size,False,direct,False)+[n for au in units for n in au])
            name=f'temporal-alt-combined-{size}-{index}'+('-direct'if direct else'')+'.mp4'
            files[name]=mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])
            cases.append(dict(file=name,size=size,direct=direct,index=index,references=refs,motion=motion))
    files['temporal-alt-combined-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC 4-layer movies, no private media, external codec, network or test-time generation.',
        specification='H.265 v8 I.8.5.3.2.7 step 9 applies AltRefIdx only when Col itself is selected.',
        probe=[32,0,32,32],first_cu_references=[1,1],first_cu_motion=[2,-2],
        probe_initial_order=['A1','Col','A1L0+ColL1','ColL0+A1L1','Zero0'],
        cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
