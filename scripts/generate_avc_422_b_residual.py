"""Own CAVLC B PCM/inter 422 residuals over weighted, reordered references."""
import json, struct
from pathlib import Path
from generate_avc_pcm_formats import Writer, mux
from generate_avc_422_b import WEIGHTS, OFFSETS
BASE=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-b'
OUT=BASE.parent/'avc-422-b-residual'
def generate(base_dir=BASE, out_dir=OUT):
    BASE,OUT=base_dir,out_dir
    OUT.mkdir(exist_ok=True); cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        if case['cabac'] or case['mode']=='skip': continue
        yd,cd=case['luma_depth'],case['chroma_depth']; mode=case['mode']; weight=case['bipred']
        base=case['name']; config=(BASE/(base+'.avcc')).read_bytes()
        source=list(struct.unpack('<3072H',(BASE/(base+'.y16')).read_bytes()))
        first=(BASE/(base+'-0.packet')).read_bytes(); future=(BASE/(base+'-1.packet')).read_bytes()
        for dc_sign in [-1,0,1]:
            for ac_mode in ['none','full','mask']:
                name=f'{base}-dc{dc_sign}-{ac_mode}'; target=source[1024:2048]
                b=Writer(); b.ue(0); b.ue(1); b.ue(0); b.u(2,4); b.u(2,4)
                b.u(int(mode!='temporal')); b.u(0); b.u(0); b.u(0)
                if weight==1:
                    b.ue(1); b.ue(1)
                    for l in range(2):
                        b.u(1); b.se(WEIGHTS[l][0]); b.se(OFFSETS[l][0]); b.u(1)
                        for p in [1,2]: b.se(WEIGHTS[l][p]); b.se(OFFSETS[l][p])
                b.se(0); b.ue(1)
                b.ue(0); b.ue(48); b.align() # B embedded I_PCM
                for at,stride,bw,depth in [(0,32,16,yd),(512,16,8,cd),(768,16,8,cd)]:
                    for y in range(16):
                        for x in range(bw):
                            index=at+y*stride+x; value=(source[index]+17)&((1<<depth)-1)
                            b.u(value,depth); target[index]=value
                code={'l0':1,'l1':2,'bi':3,'spatial':0,'temporal':0}[mode]
                b.ue(0); b.ue(code)
                for vector in (case.get('list_motion', [[0,0],[0,0]]) if code==3 else [[0,0]] if code in [1,2] else []): b.se(vector[0]); b.se(vector[1])
                pattern=32 if ac_mode!='none' else 16 if dc_sign else 0
                b.ue({0:0,16:1,32:6}[pattern])
                if pattern:
                    b.se(0)
                    for sign in [dc_sign,-dc_sign]:
                        if sign: b.u(1,2); b.u(int(sign<0)); b.u(1)
                        else: b.u(1)
                flags=[[int(ac_mode!='none')]*8 for _ in range(2)]
                if ac_mode=='mask': flags=[[v^p for v in [1,0,0,1,1,0,0,1]] for p in range(2)]
                if ac_mode!='none':
                    for p in range(2):
                        for block in range(8):
                            high=block%2==0
                            if flags[p][block]: b.u(1,6 if high else 2); b.u((p+block)%2); b.u(1)
                            elif high: b.u(3,6)
                            else: b.u(1)
                nal=b.nal(0x01); packet=len(nal).to_bytes(4,'big')+nal
                qpc=26+6*(cd-8); qpdc=qpc+3
                for p in range(2):
                    value=dc_sign*(1 if p==0 else -1)*18*16
                    dc=(value<<(qpdc//6-6)) if qpdc//6>=6 else (value+(1<<(5-qpdc//6)))>>(6-qpdc//6)
                    for y in range(16):
                        for x in range(8):
                            block=y//4*2+x//4
                            ac=(1 if (p+block)%2==0 else -1)*8*(1<<(qpc//6)) if flags[p][block] else 0
                            residual=(dc+[2,1,-1,-2][x%4]*ac+32)//64
                            at=512+p*256+y*16+8+x
                            target[at]=max(0,min((1<<cd)-1,target[at]+residual))
                for i,data in enumerate([first,future,packet]): (OUT/f'{name}-{i}.packet').write_bytes(data)
                for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,packet)],32,16,25)),('y16',struct.pack('<3072H',*(source[:1024]+target+source[2048:])) )]: (OUT/f'{name}.{ext}').write_bytes(data)
                cases.append(dict(case,name=name,dc_sign=dc_sign,ac_mode=ac_mode))
    assert len(cases)==(486 if "motion" in BASE.name else 405)
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__': generate()
