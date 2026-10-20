"""Own 422 P fields with signed rectangular DC/field AC, qpel and weights."""
import json,struct,itertools
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-field-p';OUT=ROOT/'avc-422-field-p-residual'
def flags(component,mb,mode):
    return [0]*8 if mode=='none' else [1]*8 if mode=='full' else [v^component^mb for v in [1,0,0,1,1,0,0,1]]
def packet(case,index,bottom,dc_sign,ac_mode):
    cabac=case['cabac'];weighted=case['weighted'];vector=case['motion']
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(1);b.u(int(bottom));b.u(index+2,4)
    b.u(0);b.u(1);b.ue(0);b.ue(2 if case.get("opposite_parity",False) else 1);b.ue(3)
    if weighted:b.ue(1);b.ue(2);b.u(1);b.se(3);b.se(-5);b.u(1);b.se(3);b.se(7);b.se(-2);b.se(64)
    b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(1);pattern=2 if ac_mode!='none' else int(dc_sign!=0)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
    for mb in range(2):
        if cabac:
            c.decision(11+int(mb>0),0)
            for ctx in [14,15,16]:c.decision(ctx,0)
            for component in range(2):c.mvd(component,vector[component] if mb==0 else 0,abs(vector[component]) if mb else 0)
            for ctx in ([73,74,75,76] if mb==0 else [74,74,76,76]):c.decision(ctx,0)
            c.decision(77+int(mb>0 and pattern>0),int(pattern>0))
            if pattern:c.decision(81+int(mb>0 and pattern==2),int(pattern==2));c.decision(60,0)
            if pattern:
                for component in range(2):
                    sign=dc_sign*(1 if component==0 else -1)*(-1 if mb else 1)
                    c.decision(97+int(mb>0 and dc_sign!=0),int(sign!=0))
                    if sign:c.decision(321,1);c.decision(382,1);c.decision(258,0);c.bypass(int(sign<0))
            if pattern==2:
                for component in range(2):
                    current=flags(component,mb,ac_mode);previous=flags(component,mb-1,ac_mode)
                    for block,coded in enumerate(current):
                        left=current[block-1] if block%2 else previous[block+1] if mb else 0
                        top=current[block-2] if block>=2 else 0
                        c.decision(101+left+2*top,coded)
                        if coded:c.decision(324,1);c.decision(385,1);c.decision(267,0);c.bypass((component+mb+block)%2)
            if mb==0:c.range-=2;c.renormalize()
        else:
            b.ue(0);b.ue(0);b.se(vector[0] if mb==0 else 0);b.se(vector[1] if mb==0 else 0);b.ue([0,1,6][pattern])
            if pattern:
                b.se(0)
                for component in range(2):
                    sign=dc_sign*(1 if component==0 else -1)*(-1 if mb else 1)
                    if sign:b.u(1,2);b.u(int(sign<0));b.u(1)
                    else:b.u(1)
            if pattern==2:
                for component in range(2):
                    for block,coded in enumerate(flags(component,mb,ac_mode)):
                        if coded:b.u(1,2);b.u((component+mb+block)%2);b.u(1)
                        else:b.u(1)
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate(base_dir=BASE,out_dir=OUT):
    BASE,OUT=base_dir,out_dir
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];cd=case['chroma_depth'];maximum=(1<<cd)-1;qp=26+6*(cd-8);qpdc=qp+3
        raw=(BASE/(original+'.y16')).read_bytes();all_samples=struct.unpack('<4096H',raw)
        for dc_sign,ac_mode in itertools.product([-1,0,1],['none','full','mask']):
            name=original+f'-dc{dc_sign}-{ac_mode}';target=list(all_samples[2048:]);witnesses=0
            for component in range(2):
                for y in range(32):
                    for x in range(16):
                        mb=x//8;block=(y//2%16//4)*2+x%8//4
                        sign=dc_sign*(1 if component==0 else -1)*(-1 if mb else 1);value=sign*18*16
                        dc=value<<(qpdc//6-6) if qpdc//6>=6 else (value+(1<<(5-qpdc//6)))>>(6-qpdc//6)
                        ac=(1 if (component+mb+block)%2==0 else -1)*8*(1<<(qp//6)) if flags(component,mb,ac_mode)[block] else 0
                        residual=(dc+[2,1,-1,-2][(y//2)%4]*ac+32)//64
                        at=1024+component*512+y*16+x;prediction=target[at];v=max(0,min(maximum,prediction+residual));target[at]=v
                        wrong=(dc+[2,1,-1,-2][x%4]*ac+32)//64
                        witnesses+=int(v!=max(0,min(maximum,prediction+wrong)))
            if ac_mode!='none':assert witnesses>0,'field AC must differ from frame scan'
            packets=[(BASE/f'{original}-{i}.packet').read_bytes() for i in range(2)]
            packets.extend(packet(case,i,bottom,dc_sign,ac_mode) for i,bottom in enumerate([True,False] if case['bottom_first'] else [False,True]))
            for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
            config=(BASE/(original+'.avcc')).read_bytes()
            for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,32,50)),('y16',raw[:4096]+struct.pack('<2048H',*target))]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,dc_sign=dc_sign,ac_mode=ac_mode))
    assert len(cases)==864
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
