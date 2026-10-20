"""Owned Annex I alternative temporal target reference movies, offline."""
import json
import generate_hevc_depth_iv_b as b
from generate_hevc_cqt import reference,ue,se,packed,escape,OUT
from hevc_fixture_mp4 import mux

def explicit_target(size):
    encoder,contexts=b.banks(2)
    ref=b.context(encoder,153)
    columns=size//32
    for y in range(columns):
        for x in range(columns):
            encoder.decision(contexts['split0'],False)
            encoder.decision(contexts['skip0'],False)
            encoder.decision(contexts['pred'],False)
            encoder.decision(contexts['part0'],True)
            encoder.decision(contexts['merge'],False)
            encoder.decision(contexts['ip0'],True)
            for _ in range(2):
                encoder.decision(ref,False)
                encoder.decision(contexts['mvd'],False);encoder.decision(contexts['mvd'],False)
                encoder.decision(contexts['mvp'],False)
            encoder.decision(contexts['root'],False)
            if y*columns+x+1<columns**2:b.terminal_zero(encoder)
    return encoder.finish()

def target_header(data,alternate):
    # At POC1 all lists contain past/future ST and current-AU layer3 LT.
    value='1'+ue(3)+ue(0)+'1'+f'{1:08b}'
    value+='0'+ue(1)+ue(1)+ue(0)+'1'+ue(0)+'1'+ue(0)+ue(0)+'1'
    value+='100'+'0' # only layer3 IL active, SAO disabled
    value+='1'+ue(2)+ue(2)
    # Default L0=[past,IL,future], L1=[future,past,IL].
    # With IL first, temporal source's ST needs alternate index1.
    orders=([1,0,2],[2,0,1])if alternate else([0,1,2],[0,1,2])
    for order in orders:value+='1'+''.join(f'{i:02b}'for i in order)
    value+='0'+'0'+'1'+ue(0 if alternate else 1)+ue(0) # mvd_l1_zero, cabac_init, col L0/ref0, five merges
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return ((1<<9)|(8<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def generate():
    files={}
    for size,direct in [(32,False),(128,False),(128,True)]:
        for alternate in [False,True]:
            tex,_=reference(size,5,True,flat=True)
            past,_=reference(size,5,False);future,_=reference(size,5,False,bias=97)
            for explicit in [False,True]:
                units=[[b.header(layer,0,tex if layer==0 else past,2)for layer in [0,3,5,8]],
                [b.header(layer,2,tex if layer==0 else future,2,negative=[2])for layer in [0,3,5,8]],
                [b.header(0,1,tex,2,negative=[1],positive=[1]),
                 b.header(3,1,b.source_b(size,'bi'),0,negative=[1],positive=[1]),
                 b.header(5,1,past,2,negative=[1],positive=[1]),
                 target_header(explicit_target(size)if explicit else b.target_b(size,0),alternate)]]
                stream=b''.join(b'\0\0\0\1'+n for n in b.parameters(size,False,direct,False)+[n for au in units for n in au])
                name=f'temporal-alt-ref-{size}-'+('on'if alternate else'off')+('-direct'if direct else'')+('-explicit'if explicit else'')+'.mp4'
                files[name]=mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])
    files['temporal-alt-ref-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC and MP4, no private media, FFmpeg, network or test-time generation.',
        specification='H.265 v8 I.8.3.4 and I.8.5.3.2.6; alternate current target reference classification in merge only.',
        source_motion=[1,-2],target_motion=[2,-2],target_reference_on=[1,1],target_reference_off=[0,0],
        files=list(files)),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
