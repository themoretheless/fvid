"""Owned mixed PCM/P 422 CAVLC DC/AC residuals with separate scalar samples."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
BASE=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-inter'
OUT=BASE.parent/'avc-422-inter-residual'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        base=f'422-p-y{yd}-c{cd}-e0-x0-y0';config=(BASE/(base+'.avcc')).read_bytes();first=(BASE/(base+'-0.packet')).read_bytes()
        source=list(struct.unpack('<2048H',(BASE/(base+'.y16')).read_bytes()))[:1024]
        for dc_sign in [-1,0,1]:
            for ac_mode in ['none','full','mask']:
                name=f'422-residual-y{yd}-c{cd}-dc{dc_sign}-{ac_mode}';target=source[:]
                b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
                b.ue(0);b.ue(30);b.align() # P embedded I_PCM
                for at,stride,bw,depth in [(0,32,16,yd),(512,16,8,cd),(768,16,8,cd)]:
                    for y in range(16):
                        for x in range(bw):
                            index=at+y*stride+x;value=(source[index]+17)&((1<<depth)-1);b.u(value,depth);target[index]=value
                pattern=32 if ac_mode!='none' else 16 if dc_sign else 0
                b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue({0:0,16:1,32:6}[pattern])
                if pattern:b.se(0)
                if pattern:
                    for sign in [dc_sign,-dc_sign]:
                        if sign:b.u(1,2);b.u(int(sign<0));b.u(1)
                        else:b.u(1)
                flags=[[int(ac_mode!='none')]*8 for _ in range(2)]
                if ac_mode=='mask':flags=[[v^component for v in [1,0,0,1,1,0,0,1]]for component in range(2)]
                if ac_mode!='none':
                    for component in range(2):
                        for block in range(8):
                            high=block%2==0 # left PCM16; internal top count is 0/1
                            coded=flags[component][block]
                            if coded:b.u(1,6 if high else 2);b.u((component+block)%2);b.u(1)
                            elif high:b.u(3,6)
                            else:b.u(1)
                nal=b.nal(0x41);second=len(nal).to_bytes(4,'big')+nal
                qpc=26+6*(cd-8);qpdc=qpc+3
                for component in range(2):
                    value=dc_sign*(1 if component==0 else -1)*18*16
                    dc=(value<<(qpdc//6-6))if qpdc//6>=6 else(value+(1<<(5-qpdc//6)))>>(6-qpdc//6)
                    for y in range(16):
                        for x in range(8):
                            block=y//4*2+x//4
                            ac=(1 if (component+block)%2==0 else -1)*8*(1<<(qpc//6))if flags[component][block]else 0
                            residual=(dc+[2,1,-1,-2][x%4]*ac+32)//64
                            at=512+component*256+y*16+8+x;target[at]=max(0,min((1<<cd)-1,source[at]+residual))
                for suffix,data in [('avcc',config),('-0.packet',first),('-1.packet',second),('mp4',mux(config,[(0,True,first),(1,False,second)],32,16,25)),('y16',struct.pack('<2048H',*(source+target)))]:
                    (OUT/(name+suffix if suffix.startswith('-')else name+'.'+suffix)).write_bytes(data)
                cases.append(dict(name=name,width=32,height=16,luma_depth=yd,chroma_depth=cd,dc_sign=dc_sign,ac_mode=ac_mode))
    # 32x32 4-MB CAVLC P slice: MB0=I_PCM, MB1=P_Skip, MB2=I_PCM, MB3=coded P_L0_16x16 with chroma AC.
    # For MB3, left neighbour MB2 has chroma counts=16 and top neighbour MB1 (skipped) has chroma counts=0,
    # so nC for MB3 top-left chroma AC block is (16 + 0 + 1) // 2 = 8 (VLC3).
    for yd,cd in [(8,8),(10,12)]:
        from generate_avc_pcm_formats import configuration
        name=f'422-skip-neighbour-y{yd}-c{cd}'
        config=configuration(2,yd,cd,False,cabac=False,height_mbs=2)
        source=[]
        for p,(depth,w,h)in enumerate([(yd,32,32),(cd,16,32),(cd,16,32)]):
            source.extend([(x*23+y*11+p*37)&((1<<depth)-1)for y in range(h)for x in range(w)])
        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
        for mb in range(4):
            mx,my=mb%2,mb//2;b.ue(25);b.align()
            for at,stride,bw,depth in [(0,32,16,yd),(1024,16,8,cd),(1536,16,8,cd)]:
                for y in range(16):
                    for x in range(bw):b.u(source[at+(my*16+y)*stride+mx*bw+x],depth)
        nal=b.nal(0x65);first=len(nal).to_bytes(4,'big')+nal
        target=source[:]
        b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
        for mb,run in [(0,0),(2,1)]:
            mx,my=mb%2,mb//2;b.ue(run);b.ue(30);b.align()
            for at,stride,bw,depth in [(0,32,16,yd),(1024,16,8,cd),(1536,16,8,cd)]:
                for y in range(16):
                    for x in range(bw):
                        idx=at+(my*16+y)*stride+mx*bw+x;val=(source[idx]+9)&((1<<depth)-1);b.u(val,depth);target[idx]=val
        # MB3: run=0, P_L0_16x16 (0), mvd=(0,0), CBP=32 (code 6), qp_delta=0, 2 zero chroma DC tokens, 16 zero chroma AC tokens
        b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(6);b.se(0)
        b.u(1);b.u(1)
        for _ in range(2):
            for block in range(8):
                # block 0: left=16 (MB2), top=0 (MB1 skip) -> nC=8 -> VLC3 (0,0) = 000011 (6 bits)
                # blocks 2,4,6: left=16 (MB2), top=0 (internal) -> nC=8 -> VLC3 (0,0) = 000011 (6 bits)
                # blocks 1,3,5,7: left=0 (internal), top=0 -> nC=0 -> VLC0 (0,0) = 1 (1 bit)
                if block%2==0:b.u(3,6)
                else:b.u(1,1)
        nal=b.nal(0x41);second=len(nal).to_bytes(4,'big')+nal
        for suffix,data in [('avcc',config),('-0.packet',first),('-1.packet',second),('mp4',mux(config,[(0,True,first),(1,False,second)],32,32,25)),('y16',struct.pack('<4096H',*(source+target)))]:
            (OUT/(name+suffix if suffix.startswith('-')else name+'.'+suffix)).write_bytes(data)
        cases.append(dict(name=name,width=32,height=32,luma_depth=yd,chroma_depth=cd,dc_sign=0,ac_mode='skip_neighbour'))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
