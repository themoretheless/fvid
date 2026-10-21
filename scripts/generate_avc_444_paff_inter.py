"""Owned PAFF frame pictures with shared 444 CAVLC/CABAC inter, 2x2 MB geometry."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_field_inter import configuration,WEIGHTS
from generate_avc_pcm_formats import Writer,mux
from generate_avc_monochrome_fractional import prediction
from generate_avc_444_deblock import coefficient
from generate_avc_separate_8x8_deblock import residual_8x8_single
from generate_avc_444_cabac_intra import single
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-paff-inter'
def header(index,cabac,weighted,init):
    b=Writer();b.ue(0);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u(index*2,4)
    if index==0:b.u(0);b.u(0)
    else:
        b.u(0);b.u(0)
        if weighted:
            b.ue(1 if index==1 else 0);b.ue(1 if index==1 else 0);b.u(int(index==1))
            if index==1:b.se(WEIGHTS[0][0]);b.se(WEIGHTS[0][1])
            b.u(int(index==1))
            if index==1:
                for weight,offset in WEIGHTS[1:]:b.se(weight);b.se(offset)
        b.u(0)
        if cabac:b.ue(init)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
    return b

def make(yd,cd,entropy,vector,weighted,kind):
    cabac=entropy>=0;init=max(0,entropy);depths=[yd,cd,cd]
    source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(32)for x in range(32)]for p,depth in enumerate(depths)]
    b=header(0,cabac,False,init)
    if cabac:c=Cabac444Writer()
    for mb in range(4):
        mx,my=mb%2,mb//2
        if cabac:c.decision(3+int(mx>0)+int(my>0),1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for p,depth in enumerate(depths):
            for row in range(16):
                for col in range(16):b.u(source[p][(my*16+row)*32+mx*16+col],depth)
        if cabac:
            c.low=0;c.range=510;c.width=9
            if mb<3:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x65,trailing=not cabac);packets=[len(nal).to_bytes(4,'big')+nal]
    target=[prediction(plane,depth,(0,0)if kind=='skip'else vector,height=32)for plane,depth in zip(source,depths)]
    if weighted:target=[[max(0,min((1<<depth)-1,((WEIGHTS[p][0]*v+1)//2)+(WEIGHTS[p][1]<<(depth-8))))for v in plane]for p,(plane,depth)in enumerate(zip(target,depths))]
    b=header(1,cabac,weighted,init)
    if cabac:c=Cabac444Writer(init,26)
    if not cabac and kind=='skip':b.ue(4)
    else:
        for mb in range(4):
            mx,my=mb%2,mb//2;skip=kind=='skip';residual=kind in ['ac4','dc8'];eight=kind=='dc8'
            if cabac:c.decision(11+(int(mx>0)+int(my>0)if not skip else 0),int(skip))
            else:b.ue(0)
            if not skip:
                if cabac:
                    for ctx in [14,15,16]:c.decision(ctx,0)
                    for axis in range(2):c.mvd(axis,vector[axis]if mb==0 else 0,abs(vector[axis])if mb in [1,2]else 0)
                    for block in range(4):c.decision(73 if residual else 73+int(block%2>0 or mx>0)+2*int(block//2>0 or my>0),int(residual))
                    if residual:c.decision(399+(int(mx>0)+int(my>0)if eight else 0),int(eight));c.decision(60,0)
                else:
                    b.ue(0);b.se(vector[0]if mb==0 else 0);b.se(vector[1]if mb==0 else 0);b.ue(9 if residual else 0)
                    if residual:b.u(int(eight));b.se(0)
                if residual:
                    for p,depth in enumerate(depths):
                        for block in range(4 if eight else 16):
                            sign=-1 if(p+mb+block)%2 else 1;side=8 if eight else 4
                            if eight:bx,by=block%2*8,block//2*8
                            else:bx,by=((block&1)+((block>>2)&1)*2)*4,(((block>>1)&1)+(block>>3)*2)*4
                            if cabac:
                                inc=int(bx>0 or mx>0)+2*int(by>0 or my>0)
                                if eight:single(c,[5,9,13][p],inc,4*sign)
                                else:
                                    cbf=[93,468,480][p];sig=[134,513,557][p];last=[195,601,645][p];absolute=[247,972,1002][p]
                                    for ctx,v in [(cbf+inc,1),(sig,0),(sig+1,1),(last+1,1),(absolute+1,0)]:c.decision(ctx,v)
                                    c.bypass(int(sign<0))
                            elif eight:
                                for sub in range(4):
                                    if sub==0:coefficient(b,4*sign,0)
                                    else:b.u(1)
                            else:coefficient(b,sign,1)
                            if eight:res=residual_8x8_single(0,4*sign,26,depth,16)
                            else:
                                q=26+6*(depth-8);v=sign*16*[13,14,16,18,20,23][q%6];scaled=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6);r=[(v+32)>>6 for v in [scaled,scaled>>1,-(scaled>>1),-scaled]];res=[r[x]for y in range(4)for x in range(4)]
                            for y in range(side):
                                for x in range(side):at=(my*16+by+y)*32+mx*16+bx+x;target[p][at]=max(0,min((1<<depth)-1,target[p][at]+res[y*side+x]))
            if cabac and mb<3:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac);packets.append(len(nal).to_bytes(4,'big')+nal)
    b=header(2,cabac,weighted,init)
    if cabac:
        c=Cabac444Writer(init,26)
        for mb in range(4):
            c.decision(11,1)
            if mb<3:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
    else:b.ue(4)
    nal=b.nal(0x41,trailing=not cabac);packets.append(len(nal).to_bytes(4,'big')+nal)
    return configuration(yd,cd,cabac,weighted),packets,sum(source+target+target,[])

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for(yd,cd),entropy,vector,weighted,kind in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],[(0,0),(-3,2)],[False,True],['skip','mc','ac4','dc8']):
        name=f'paff-y{yd}-c{cd}-e{entropy}-x{vector[0]}-y{vector[1]}-w{int(weighted)}-{kind}';cfg,packets,expected=make(yd,cd,entropy,vector,weighted,kind)
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,32,25)),('y16',struct.pack('<9216H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,vector=vector,weighted=weighted,kind=kind))
    assert len(cases)==192
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
