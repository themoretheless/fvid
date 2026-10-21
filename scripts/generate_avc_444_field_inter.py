"""Owned shared 444 P fields: parity, mixed depth, weighted qpel and nonzero field scans."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_fields import configuration as base_config
from generate_avc_pcm_formats import Writer,mux
from generate_avc_monochrome_fractional import prediction
from generate_avc_444_deblock import coefficient
from generate_avc_separate_8x8_deblock import residual_8x8_single
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-field-inter'
WEIGHTS=[(3,-5),(1,2),(-1,1)]
def configuration(yd,cd,cabac,weighted,bipred=0,offsets=(0,0)):
    cfg=base_config(yd,cd,cabac,[0,0],height_map_units=1)
    n=int.from_bytes(cfg[6:8],'big');b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0);b.u(1);b.u(0);b.se(offsets[1]);pps=b.nal(0x68)
    return cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps

def field_header(index,bottom,initial,cabac,weighted,opposite,init,poc_base=2,filter_mode=1):
    b=Writer();b.ue(0);b.ue(2 if initial else 0);b.ue(0);b.u(0 if initial else 1,4);b.u(1);b.u(int(bottom))
    if initial and index==0:b.ue(0)
    b.u(index if initial else index+poc_base,4)
    if initial:
        b.u(0)
        if index==0:b.u(0)
    else:
        b.u(0);b.u(1);b.ue(0);b.ue(2 if opposite else 1);b.ue(3)
        if weighted:
            b.ue(1);b.ue(1);b.u(1);b.se(WEIGHTS[0][0]);b.se(WEIGHTS[0][1]);b.u(1)
            for weight,offset in WEIGHTS[1:]:b.se(weight);b.se(offset)
        b.u(0)
        if cabac:b.ue(init)
    b.se(0);b.ue(filter_mode)
    if filter_mode!=1:b.se(0);b.se(0)
    if cabac:
        while len(b.bits)%8:b.u(1)
    return b

def pcm(index,bottom,planes,depths,cabac):
    b=field_header(index,bottom,True,cabac,False,False,0)
    if cabac:c=Cabac444Writer()
    for mb in range(2):
        if cabac:c.decision(3+int(mb>0),1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for p,depth in enumerate(depths):
            for row in range(16):
                for col in range(16):b.u(planes[p][(row*2+int(bottom))*32+mb*16+col],depth)
        if cabac:
            c.low=0;c.range=510;c.width=9
            if mb==0:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def field_single(c,p,eight,inc,sign):
    cbf=([1012,1016,1020]if eight else[93,468,480])[p]
    sig=([436,675,733]if eight else[306,805,849])[p]
    last=([451,699,757]if eight else[367,893,937])[p]
    absolute=([426,708,766]if eight else[247,972,1002])[p]
    c.decision(cbf+inc,1)
    if eight:
        for pos,v in enumerate([0,0,0,1]):c.decision(sig+[0,1,1,2][pos],v)
        c.decision(last+1,1);c.decision(absolute+1,1)
        for v in [1,1,0]:c.decision(absolute+5,v)
    else:c.decision(sig,0);c.decision(sig+1,1);c.decision(last+1,1);c.decision(absolute+1,0)
    c.bypass(int(sign<0))

def p_field(index,bottom,cabac,vector,kind,weighted,opposite,init,poc_base=2):
    b=field_header(index,bottom,False,cabac,weighted,opposite,init,poc_base=poc_base)
    if cabac:c=Cabac444Writer(init,26)
    if not cabac and kind=='skip':b.ue(2)
    else:
        for mb in range(2):
            skip=kind=='skip';residual=kind in ['ac4','ac8'];eight=kind=='ac8'
            if cabac:c.decision(11+int(mb>0 and not skip),int(skip))
            else:b.ue(0)
            if not skip:
                if cabac:
                    for ctx in [14,15,16]:c.decision(ctx,0)
                    for axis in range(2):c.mvd(axis,vector[axis]if mb==0 else 0,abs(vector[axis])if mb else 0)
                    for ctx in ([73]*4 if residual else[73,74,75,76]if mb==0 else[74,74,76,76]):c.decision(ctx,int(residual))
                    if residual:c.decision(399+int(eight and mb>0),int(eight));c.decision(60,0)
                else:
                    b.ue(0);b.se(vector[0]if mb==0 else 0);b.se(vector[1]if mb==0 else 0);b.ue(9 if residual else 0)
                    if residual:b.u(int(eight));b.se(0)
                if residual:
                    for p in range(3):
                        for block in range(4 if eight else 16):
                            sign=-1 if(p+mb+block)%2 else 1
                            if cabac:
                                if eight:x,y=block%2,block//2
                                else:x,y=(block&1)+((block>>2)&1)*2,((block>>1)&1)+(block>>3)*2
                                field_single(c,p,eight,int(x>0 or mb>0)+2*int(y>0),sign)
                            elif eight:
                                for sub in range(4):
                                    if sub==3:coefficient(b,4*sign,0)
                                    else:b.u(1)
                            else:coefficient(b,sign,1)
            if cabac and mb==0:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def make(yd,cd,entropy,reverse,opposite,vector,weighted,kind):
    cabac=entropy>=0;init=max(0,entropy);depths=[yd,cd,cd]
    planes=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(32)for x in range(32)]for p,depth in enumerate(depths)]
    order=[True,False]if reverse else[False,True];packets=[pcm(i,bottom,planes,depths,cabac)for i,bottom in enumerate(order)];fields={};witness=0
    for index,bottom in enumerate(order):
        packets.append(p_field(index,bottom,cabac,vector,kind,weighted,opposite,init));components=[]
        for p,depth in enumerate(depths):
            parity=int(not bottom if opposite else bottom)
            compact=[v for y in range(16)for v in planes[p][(y*2+parity)*32:(y*2+parity+1)*32]]
            target=prediction(compact,depth,(0,0)if kind=='skip'else vector)
            if weighted:target=[max(0,min((1<<depth)-1,((WEIGHTS[p][0]*v+1)//2)+(WEIGHTS[p][1]<<(depth-8))))for v in target]
            if kind in ['ac4','ac8']:
                for mb in range(2):
                    for block in range(4 if kind=='ac8'else 16):
                        sign=-1 if(p+mb+block)%2 else 1;side=8 if kind=='ac8'else 4
                        if side==8:
                            bx,by=block%2*8,block//2*8;res=residual_8x8_single(1,4*sign,26,depth,16);wrong=residual_8x8_single(16,4*sign,26,depth,16)
                        else:
                            bx=((block&1)+((block>>2)&1)*2)*4;by=(((block>>1)&1)+(block>>3)*2)*4
                            q=26+6*(depth-8);v=sign*16*[13,14,16,18,20,23][q%6];scaled=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6)
                            r=[(v+32)>>6 for v in [scaled,scaled>>1,-(scaled>>1),-scaled]];res=[r[y]for y in range(4)for x in range(4)];wrong=[r[x]for y in range(4)for x in range(4)]
                        for y in range(side):
                            for x in range(side):
                                at=(by+y)*32+mb*16+bx+x;old=target[at];value=max(0,min((1<<depth)-1,old+res[y*side+x]));witness+=int(value!=max(0,min((1<<depth)-1,old+wrong[y*side+x])));target[at]=value
            components.append(target)
        fields[bottom]=components
    if kind in ['ac4','ac8']:assert witness>0
    woven=[]
    for p in range(3):
        for y in range(16):
            for bottom in [False,True]:woven+=fields[bottom][p][y*32:(y+1)*32]
    return configuration(yd,cd,cabac,weighted),packets,sum(planes,[])+woven,witness

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={'ac4':0,'ac8':0}
    for(yd,cd),entropy,reverse,opposite,vector,weighted,kind in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],[False,True],[False,True],[(0,0),(1,-3)],[False,True],['skip','mc','ac4','ac8']):
        name=f'field-p-y{yd}-c{cd}-e{entropy}-r{int(reverse)}-o{int(opposite)}-x{vector[0]}-y{vector[1]}-w{int(weighted)}-{kind}'
        cfg,packets,expected,count=make(yd,cd,entropy,reverse,opposite,vector,weighted,kind)
        if kind in witness:witness[kind]+=count
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,32,50)),('y16',struct.pack('<6144H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,reverse=reverse,opposite=opposite,vector=vector,weighted=weighted,kind=kind))
    assert len(cases)==768 and all(v>0 for v in witness.values())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
