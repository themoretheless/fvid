"""CABAC author for the independent owned 422 intra scalar sample matrix."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
BASE=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-intra'
OUT=BASE.parent/'avc-422-cabac'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    originals=json.loads((BASE/'manifest.json').read_text())
    settings=originals+[dict(c,ac_mask=m)for c in originals if c['mode']==1 and c['sign']==0 and c['luma']=='i16' and c['ac'] for m in [0,1]]
    for case in settings:
        yd,cd=case['luma_depth'],case['chroma_depth'];mode,sign,ac,luma=case['mode'],case['sign'],case['ac'],case['luma']
        name='cabac-'+case['name']+(f"-mask{case['ac_mask']}"if'ac_mask'in case else'');config=configuration(2,yd,cd,False,True,height_mbs=2)
        planes=[[(x*17+y*23+c*43)&((1<<dep)-1) for y in range(32)for x in range(w)]for c,(dep,w)in enumerate([(yd,32),(cd,16),(cd,16)])]
        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(-1,26)
        for mb in range(3):
            c.decision(3 if mb==0 else 4,1);b.bits.extend(c.finish());b.align()
            for p,(depth,stride,bw)in enumerate([(yd,32,16),(cd,16,8),(cd,16,8)]):
                for y in range(16):
                    for x in range(bw):b.u(planes[p][(mb//2*16+y)*stride+mb%2*bw+x],depth)
            c.low=0;c.range=510;c.width=9;c.range-=2;c.renormalize() # PCM end_of_slice_flag0, contexts retained
        pattern=2 if ac else int(sign!=0)
        c.decision(5,int(luma=='i16'))
        if luma=='i16':
            c.range-=2;c.renormalize();c.decision(6,0);c.decision(7,int(pattern>0))
            if pattern:c.decision(8,int(pattern==2))
            c.decision(9,1);c.decision(10,0)
        else:
            for _ in range(16):c.decision(68,1)
        c.decision(64,int(mode!=0))
        if mode:
            c.decision(67,int(mode>1))
            if mode>1:c.decision(67,int(mode==3))
        if luma=='i4':
            for ctx in [73,74,75,76]:c.decision(ctx,0)
            c.decision(80,int(pattern>0))
            if pattern:c.decision(84,int(pattern==2))
        if luma=='i16' or pattern:c.decision(60,0)
        if luma=='i16':c.decision(88,0)
        if pattern:
            for s in [sign,-sign]:
                c.decision(100,int(s!=0))
                if s:
                    c.decision(149,1);c.decision(210,1);c.decision(258,0);c.bypass(int(s<0))
        flags=[[1]*8 for _ in range(2)]
        if 'ac_mask'in case:
            flags=[[v^case['ac_mask']^component for v in [1,0,0,1,1,0,0,1]]for component in range(2)]
        context_witnesses=0
        if ac:
            for component in range(2):
                for block in range(8):
                    left=flags[component][block-1]if block%2 else 1
                    top=flags[component][block-2]if block>=2 else 1
                    inc=left+2*top;context_witnesses+=int(inc!=3)
                    coded=flags[component][block];c.decision(101+inc,coded)
                    if coded:c.decision(152,1);c.decision(213,1);c.decision(267,0);c.bypass((component+block)%2)
        b.bits.extend(c.finish());nal=b.nal(0x65,trailing=False);packet=len(nal).to_bytes(4,'big')+nal
        golden=(BASE/(case['name']+'.y16')).read_bytes()
        if 'ac_mask'in case:
            assert context_witnesses>0
            raw=(BASE/(case['name'].removesuffix('-ac')+'.y16')).read_bytes();values=list(struct.unpack('<2048H',raw))
            for component in range(2):
                for y in range(16):
                    for x in range(8):
                        block=y//4*2+x//4
                        if flags[component][block]:
                            amplitude=(1 if (component+block)%2==0 else -1)*8*(1<<((26+6*(cd-8))//6))
                            residual=([2,1,-1,-2][x%4]*amplitude+32)//64
                            at=1024+component*512+(16+y)*16+8+x
                            values[at]=max(0,min((1<<cd)-1,values[at]+residual))
            golden=struct.pack('<2048H',*values)
        for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,32,25)),('y16',golden)]: (OUT/(name+'.'+suffix)).write_bytes(data)
        cases.append(dict(case,name=name,context_witnesses=context_witnesses))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
