"""Own single-slice P filtering counterparts of the independent border oracle."""
import json
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-inter-deblock';OUT=ROOT/'avc-422-inter-deblock-single'
def packet(cabac,axis,motion_delta,mode):
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
    for mb in range(4):
        vector=[0,0];vector[axis]=motion_delta*[mb%2,mb//2][axis]
        # All four 16x16 median predictors are zero for this authored grid.
        if cabac:
            c.decision(11+int(mb%2>0)+int(mb>=2),0)
            for ctx in [14,15,16]:c.decision(ctx,0)
            for component in range(2):c.mvd(component,vector[component],motion_delta if mb==3 and component==axis else 0)
            for block in range(4):
                left=block%2>0 or mb%2>0;top=block>=2 or mb>=2
                c.decision(73+int(left)+2*int(top),0)
            c.decision(77,0)
            if mb!=3:c.range-=2;c.renormalize()
        else:b.ue(0);b.ue(0);b.se(vector[0]);b.se(vector[1]);b.ue(0)
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];name='single-'+original;mode=case['filter_mode']
        config=(BASE/(original+'.avcc')).read_bytes();first=(BASE/(original+'-0.packet')).read_bytes()
        second=packet(case['cabac'],case['axis'],case['motion_delta'],mode)
        # Modes 0 and 2 have the same effect inside one slice. The source oracle
        # independently calculates bS=1 deltas; bS=0 controls remain unchanged.
        oracle=original.replace('-m2','-m0') if mode==2 else original
        raw=(BASE/(oracle+'.y16')).read_bytes()
        for i,data in enumerate([first,second]):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(1,False,second)],32,32,25)),('y16',raw)]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(case,name=name,single_slice=True))
    assert len(cases)==72
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
