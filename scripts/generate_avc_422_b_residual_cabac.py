"""Owned CABAC B PCM/inter residuals paired with the independent scalar oracle."""
import json, struct
from pathlib import Path
from generate_avc_pcm_formats import Writer, mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
from generate_avc_422_b import WEIGHTS, OFFSETS
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-b'; SCALAR=ROOT/'avc-422-b-residual'; OUT=ROOT/'avc-422-b-residual-cabac'
def generate(scalar_dir=SCALAR, base_dir=BASE, out_dir=OUT):
    SCALAR,BASE,OUT=scalar_dir,base_dir,out_dir
    OUT.mkdir(exist_ok=True); cases=[]
    for case in json.loads((SCALAR/'manifest.json').read_text()):
        yd,cd=case['luma_depth'],case['chroma_depth']; mode=case['mode']; weight=case['bipred']
        dc_sign,ac_mode=case['dc_sign'],case['ac_mode']; name='cabac-'+case['name']
        base=case['name'].split('-dc')[0].replace('-e0-', '-e1-')
        config=(BASE/(base+'.avcc')).read_bytes()
        first=(BASE/(base+'-0.packet')).read_bytes(); future=(BASE/(base+'-1.packet')).read_bytes()
        raw=(SCALAR/(case['name']+'.y16')).read_bytes(); samples=struct.unpack('<3072H',raw)
        b=Writer(); b.ue(0); b.ue(1); b.ue(0); b.u(2,4); b.u(2,4)
        b.u(int(mode!='temporal')); b.u(0); b.u(0); b.u(0)
        if weight==1:
            b.ue(1); b.ue(1)
            for l in range(2):
                b.u(1); b.se(WEIGHTS[l][0]); b.se(OFFSETS[l][0]); b.u(1)
                for p in [1,2]: b.se(WEIGHTS[l][p]); b.se(OFFSETS[l][p])
        b.ue(0); b.se(0); b.ue(1)
        while len(b.bits)%8: b.u(1)
        c=CabacWriter(0,26); c.decision(24,0); mb_type(c,23,0); c.decision(32,1)
        b.bits.extend(c.finish()); b.align()
        for at,stride,bw,depth in [(0,32,16,yd),(512,16,8,cd),(768,16,8,cd)]:
            for y in range(16):
                for x in range(bw): b.u(samples[1024+at+y*stride+x],depth)
        c.low=0; c.range=510; c.width=9; c.range-=2; c.renormalize()
        c.decision(25,0)
        code={'l0':1,'l1':2,'bi':3,'spatial':0,'temporal':0}[mode]
        mb_type(c,code,1)
        for vector in (case.get('list_motion', [[0,0],[0,0]]) if code==3 else [[0,0]] if code in [1,2] else []): c.mvd(0,vector[0]); c.mvd(1,vector[1])
        for ctx in [73,74,75,76]: c.decision(ctx,0)
        pattern=2 if ac_mode!='none' else int(dc_sign!=0)
        c.decision(78,int(pattern>0))
        if pattern: c.decision(82,int(pattern==2)); c.decision(60,0)
        if pattern:
            for sign in [dc_sign,-dc_sign]:
                c.decision(98,int(sign!=0))
                if sign: c.decision(149,1); c.decision(210,1); c.decision(258,0); c.bypass(int(sign<0))
        flags=[[int(ac_mode!='none')]*8 for _ in range(2)]
        if ac_mode=='mask': flags=[[v^p for v in [1,0,0,1,1,0,0,1]] for p in range(2)]
        if pattern==2:
            for p in range(2):
                for block in range(8):
                    left=flags[p][block-1] if block%2 else 1
                    top=flags[p][block-2] if block>=2 else 0
                    coded=flags[p][block]; c.decision(101+left+2*top,coded)
                    if coded: c.decision(152,1); c.decision(213,1); c.decision(267,0); c.bypass((p+block)%2)
        b.bits.extend(c.finish()); nal=b.nal(0x01,trailing=False); packet=len(nal).to_bytes(4,'big')+nal
        for i,data in enumerate([first,future,packet]): (OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,packet)],32,16,25)),('y16',raw)]: (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(case,name=name,cabac=True))
    assert len(cases)==(486 if "motion" in BASE.name else 405)
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__': generate()
