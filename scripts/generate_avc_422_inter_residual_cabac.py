"""Own CABAC P-PCM/P16x16 residuals paired with independent CAVLC scalar samples."""
import json
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
BASE=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-inter'
SCALAR=BASE.parent/'avc-422-inter-residual';OUT=BASE.parent/'avc-422-inter-residual-cabac'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((SCALAR/'manifest.json').read_text()):
        yd,cd=case['luma_depth'],case['chroma_depth'];dc_sign,ac_mode=case['dc_sign'],case['ac_mode']
        base=f'422-p-y{yd}-c{cd}-e1-x0-y0';name='cabac-'+case['name'];config=(BASE/(base+'.avcc')).read_bytes();first=(BASE/(base+'-0.packet')).read_bytes()
        planes=[[(x*1703+y*7907+x*y*113+p*43)&((1<<depth)-1)for y in range(16)for x in range(w)]for p,(depth,w)in enumerate([(yd,32),(cd,16),(cd,16)])]
        b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.ue(0);b.se(0);b.ue(1)
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26);c.decision(11,0);c.decision(14,1);c.decision(17,1);b.bits.extend(c.finish());b.align()
        for p,(depth,w,bw)in enumerate([(yd,32,16),(cd,16,8),(cd,16,8)]):
            for y in range(16):
                for x in range(bw):b.u((planes[p][y*w+x]+17)&((1<<depth)-1),depth)
        c.low=0;c.range=510;c.width=9;c.range-=2;c.renormalize()
        for ctx,value in [(12,0),(14,0),(15,0),(16,0),(40,0),(47,0),(73,0),(74,0),(75,0),(76,0)]:c.decision(ctx,value)
        pattern=2 if ac_mode!='none'else int(dc_sign!=0)
        c.decision(78,int(pattern>0))
        if pattern:c.decision(82,int(pattern==2));c.decision(60,0)
        if pattern:
            for sign in [dc_sign,-dc_sign]:
                c.decision(98,int(sign!=0))
                if sign:c.decision(149,1);c.decision(210,1);c.decision(258,0);c.bypass(int(sign<0))
        flags=[[int(ac_mode!='none')]*8 for _ in range(2)]
        if ac_mode=='mask':flags=[[v^component for v in [1,0,0,1,1,0,0,1]]for component in range(2)]
        if pattern==2:
            for component in range(2):
                for block in range(8):
                    left=flags[component][block-1]if block%2 else 1
                    top=flags[component][block-2]if block>=2 else 0
                    coded=flags[component][block];c.decision(101+left+2*top,coded)
                    if coded:c.decision(152,1);c.decision(213,1);c.decision(267,0);c.bypass((component+block)%2)
        b.bits.extend(c.finish());nal=b.nal(0x41,trailing=False);second=len(nal).to_bytes(4,'big')+nal
        for suffix,data in [('avcc',config),('-0.packet',first),('-1.packet',second),('mp4',mux(config,[(0,True,first),(1,False,second)],32,16,25)),('y16',(SCALAR/(case['name']+'.y16')).read_bytes())]: (OUT/(name+suffix if suffix.startswith('-')else name+'.'+suffix)).write_bytes(data)
        cases.append(dict(case,name=name))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
