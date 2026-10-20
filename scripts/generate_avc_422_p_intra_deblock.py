"""Owned P-slice intra bS=3/4 counterpart of scalar 422 filtering oracle."""
import json
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-deblock';REF=ROOT/'avc-422-inter-deblock';OUT=ROOT/'avc-422-p-intra-deblock'
def packet(case):
    cabac=case['cabac'];mode=case['mode'];internal=case['internal'];vertical=case['vertical'];nals=[]
    for address in range(4):
        sign=1 if internal or (address%2 if vertical else address//2) else -1
        b=Writer();b.ue(address);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0)
        if cabac:b.ue(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        if not cabac:
            b.ue(0);b.ue(12);b.ue(0);b.se(0);b.u(1) # P embedded I16 DC, CBP16, Y DC0
            for s in [sign,-sign]:b.u(15,7);b.u(1,6 if s<0 else 5);b.u(2,4) if internal else b.u(1)
        else:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,26);c.decision(11,0);c.decision(14,1);c.decision(17,1);c.range-=2;c.renormalize()
            for ctx,value in [(18,0),(19,1),(19,0),(20,1),(20,0),(64,0),(60,0),(88,0)]:c.decision(ctx,value)
            for s in [sign,-sign]:
                c.decision(100,1)
                if internal:
                    for ctx,value in [(149,0),(149,0),(150,0),(150,1),(211,1)]:c.decision(ctx,value)
                else:c.decision(149,1);c.decision(210,1)
                for ctx,value in [(258,1),(262,1),(262,1),(262,0)]:c.decision(ctx,value)
                c.bypass(int(s<0))
            b.bits.extend(c.finish())
        nals.append(b.nal(0x41,trailing=not cabac))
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];name='p-intra-'+original;yd,cd=case['luma_depth'],case['chroma_depth'];e=int(case['cabac'])
        ref=f'422-inter-filter-y{yd}-c{cd}-e{e}-a0-m1'
        config=(BASE/(original+'.avcc')).read_bytes();first=(REF/(ref+'-0.packet')).read_bytes();second=packet(case)
        raw=(REF/(ref+'.y16')).read_bytes()[:4096]+(BASE/(original+'.y16')).read_bytes()
        for i,data in enumerate([first,second]):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(1,False,second)],32,32,25)),('y16',raw)]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(case,name=name))
    assert len(cases)==54
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
