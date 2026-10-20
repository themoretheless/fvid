"""Own single-slice B bi filtering, both motion lists and mode-2 acceptance."""
import json
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-b-deblock';OUT=ROOT/'avc-422-b-deblock-single'
def packet(cabac,axis,motion_delta,mode,changed_list):
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(2,4);b.u(2,4);b.u(1);b.u(0);b.u(0);b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
    for mb in range(4):
        vectors=[[0,0],[0,0]];vectors[changed_list][axis]=motion_delta*[mb%2,mb//2][axis]
        if cabac:
            inc=int(mb%2>0)+int(mb>=2)
            c.decision(24+inc,0);mb_type(c,3,inc)
            for l,vector in enumerate(vectors):
                for component in range(2):c.mvd(component,vector[component],motion_delta if mb==3 and l==changed_list and component==axis else 0)
            for block in range(4):c.decision(73+int(block%2>0 or mb%2>0)+2*int(block>=2 or mb>=2),0)
            c.decision(77,0)
            if mb!=3:c.range-=2;c.renormalize()
        else:
            b.ue(0);b.ue(3)
            for v in vectors:b.se(v[0]);b.se(v[1])
            b.ue(0)
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x01,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];name='single-'+original;mode=case['filter_mode']
        config=(BASE/(original+'.avcc')).read_bytes()
        first=(BASE/(original+'-0.packet')).read_bytes();future=(BASE/(original+'-1.packet')).read_bytes()
        middle=packet(case['cabac'],case['axis'],case['motion_delta'],mode,case['changed_list'])
        oracle=original.replace('-m2','-m0') if mode==2 else original
        raw=(BASE/(oracle+'.y16')).read_bytes()
        for i,data in enumerate([first,future,middle]):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,middle)],32,32,25)),('y16',raw)]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(case,name=name,single_slice=True))
    assert len(cases)==144
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
