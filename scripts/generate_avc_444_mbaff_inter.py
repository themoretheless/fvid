"""Owned shared 444 MBAFF P: mixed layouts, parity, weighted qpel, residual scans and constrained intra."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_fields import configuration as base_config
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import Writer,mux
from generate_avc_444_inter import WEIGHTS
from generate_avc_422_mbaff_p_fractional import luma
from generate_avc_444_deblock import coefficient
from generate_avc_separate_8x8_deblock import residual_8x8_single,write_constant_list
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-inter'
def configuration(yd,cd,cabac,weighted,constrained=False,offsets=(0,0),bipred=0,bottom_poc=False,scaling=None):
    cfg=base_config(yd,cd,cabac,offsets,mbaff=True);n=int.from_bytes(cfg[6:8],'big')
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(int(bottom_poc));b.ue(0);b.ue(0);b.ue(0);b.u(int(weighted));b.u(bipred,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(int(constrained));b.u(0);b.u(1);b.u(int(scaling is not None))
    if scaling is not None:
        for weight in [16,16,16,*scaling[0]]:write_constant_list(b,16,weight)
        for weight in [16,scaling[1][0],16,scaling[1][1],16,scaling[1][2]]:write_constant_list(b,64,weight)
    b.se(offsets[1]);pps=b.nal(0x68)
    return cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps

def header(pair,index,cabac,weighted,init,mode=1,poc=None,bottom_delta=None,intra=False):
    b=Writer();b.ue(pair);b.ue(2 if index==0 or intra else 0);b.ue(0);b.u(index,4);b.u(0)
    if index==0:b.ue(0)
    b.u(index*2 if poc is None else poc,4)
    if bottom_delta is not None:b.se(bottom_delta)
    if index==0:b.u(0);b.u(0)
    elif intra:b.u(0)
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
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    if cabac:
        while len(b.bits)%8:b.u(1)
    return b

def pcm(source,depths,topology,cabac,index=0,poc=None,bottom_delta=None):
    nals=[]
    for pair,field in enumerate(MODES[topology]):
        b=header(pair,index,cabac,False,0,poc=poc,bottom_delta=bottom_delta,intra=True)
        if cabac:c=Cabac444Writer();c.decision(70,int(field))
        for parity in range(2):
            if cabac:c.decision(3+int(parity==1 and not field),1);b.bits.extend(c.finish());b.align()
            else:
                if parity==0:b.u(int(field))
                b.ue(25);b.align()
            for p,depth in enumerate(depths):
                for row in range(16):
                    y=pair//2*32+(row*2+parity if field else row+parity*16)
                    for col in range(16):b.u(source[p][y*32+pair%2*16+col],depth)
            if cabac:c.low=0;c.range=510;c.width=9
        if cabac:b.bits.extend(c.finish())
        nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac);nals.append(nal)
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

def single(c,p,eight,field,inc,sign,zero=False):
    cbf=([1012,1016,1020]if eight else[93,468,480])[p];c.decision(cbf+inc,int(not zero))
    if zero:return
    sig=(([436,675,733]if field else[402,660,718])if eight else([306,805,849]if field else[134,513,557]))[p]
    last=(([451,699,757]if field else[417,690,748])if eight else([367,893,937]if field else[195,601,645]))[p]
    absolute=([426,708,766]if eight else[247,972,1002])[p]
    if eight:
        for pos,v in enumerate([0,0,0,1]):c.decision(sig+([0,1,1,2]if field else[0,1,2,3])[pos],v)
        c.decision(last+1,1);c.decision(absolute+1,1)
        for v in [1,1,0]:c.decision(absolute+5,v)
    else:c.decision(sig,0);c.decision(sig+1,1);c.decision(last+1,1);c.decision(absolute+1,0)
    c.bypass(int(sign<0))

def packet(topology,cabac,init,vector,kind,weighted,opposite,index=1,mode=1,poc=None,components=3,active=None,bottom_delta=None):
    active=[True]*components if active is None else active
    assert len(active)==components
    nals=[]
    for pair,field in enumerate(MODES[topology]):
        b=header(pair,index,cabac,weighted,init,mode,poc,bottom_delta=bottom_delta)
        if cabac:c=Cabac444Writer(init,26)
        for parity in range(2):
            bottom=parity==1 and not field;skip=kind=='skip-bottom'and parity==1;intra=kind=='mixed'and parity==1
            if cabac:
                c.decision(11+int(bottom),int(skip))
                if parity==0:c.decision(70,int(field))
            else:
                b.ue(int(skip))
                if parity==0:b.u(int(field))
            if skip:continue
            if intra:
                if cabac:
                    c.decision(14,1);c.decision(17,1);c.range-=2;c.renormalize()
                    for ctx,v in [(18,0),(19,0),(20,1),(20,0),(60,0)]:c.decision(ctx,v)
                    for cbf in [85,460,472][:components]:c.decision(cbf+1+2*int(not bottom),0)
                else:
                    b.ue(8);b.se(0)
                    for _ in range(components):b.u(1)
                continue
            difference=(0,0)if bottom else vector;residual=kind in ['ac4','ac8','mixed'];eight=kind=='ac8'
            if cabac:
                for ctx in [14,15,16]:c.decision(ctx,0)
                if field:
                    c.decision(54,int(opposite))
                    if opposite:c.decision(58,0)
                for axis,v in enumerate(difference):c.mvd(axis,v,abs(vector[axis])if bottom else 0)
                for ctx in ([73]*4 if residual else[75,76,75,76]if bottom else[73,74,75,76]):c.decision(ctx,int(residual))
                if residual:c.decision(399+int(bottom and eight),int(eight));c.decision(60,0)
            else:
                b.ue(0)
                if field:b.u(int(not opposite))
                b.se(difference[0]);b.se(difference[1]);b.ue(9 if residual else 0)
                if residual:b.u(int(eight));b.se(0)
            if residual:
                for p in range(components):
                    for block in range(4 if eight else 16):
                        sign=-1 if(p+pair+parity+block)%2 else 1
                        if cabac:
                            if eight:x,y=block%2,block//2
                            else:x,y=(block&1)+((block>>2)&1)*2,((block>>1)&1)+(block>>3)*2
                            single(c,p,eight,field,(int(x>0)+2*int(y>0 or bottom))if active[p]else 0,sign,zero=not active[p])
                        elif not active[p]:
                            for _ in range(4 if eight else 1):b.u(1)
                        elif eight:
                            for sub in range(4):
                                if sub==3:coefficient(b,4*sign,0)
                                else:b.u(1)
                        else:coefficient(b,sign,1)
        if cabac:b.bits.extend(c.finish())
        nal=b.nal(0x41,trailing=not cabac);nals.append(nal)
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

def expected(source,depths,topology,vector,kind,weighted,opposite,constrained,nearest=False,pair_vectors=None,active=None,qps=None,cell_vectors=None,cell_references=None,block_active=None,scaling4=(16,16,16),scaling8=(16,16,16)):
    active=[True]*len(depths)if active is None else active;qps=[26]*len(depths)if qps is None else qps
    result=[];scan_witness=0
    for p,depth in enumerate(depths):
        target=[None]*2048;maximum=(1<<depth)-1
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1
                if kind=='mixed'and parity==1:
                    dc=(sum(target[(oy-1)*32+ox+x]for x in range(16))+8)//16 if not field and not constrained else 1<<(depth-1)
                    for y in range(16):target[(oy+y*step)*32+ox:(oy+y*step)*32+ox+16]=[dc]*16
                    continue
                skip=kind=='skip-bottom'and parity==1;mv=(0,0)if skip else vector if pair_vectors is None else pair_vectors[pair];selected=parity^int(opposite and not skip)
                def sample(x,y):
                    x=max(0,min(31,x));y=max(0,min(31 if field else 63,y));return source[p][(y*2+selected if field else y)*32+x]
                logical_y=pair//2*16 if field else oy
                for row in range(16):
                    for col in range(16):
                        if cell_vectors is not None:mv=cell_vectors[(pair*2+parity,col//4,row//4)]
                        if cell_references is not None:selected=parity^(cell_references[(pair*2+parity,col//4,row//4)]%2)
                        x=ox+col+mv[0]//4;y=logical_y+row+mv[1]//4
                        value=sample(x,y)if nearest else luma(sample,maximum,x,y,mv[0]%4,mv[1]%4)
                        if weighted:value=max(0,min(maximum,((WEIGHTS[p][0]*value+1)//2)+(WEIGHTS[p][1]<<(depth-8))))
                        target[(oy+row*step)*32+ox+col]=value
                if active[p]and kind in ['ac4','ac8','mixed']:
                    eight=kind=='ac8';side=8 if eight else 4
                    for block in range(4 if eight else 16):
                        if block_active is not None and not block_active(p,pair*2+parity,block,eight):continue
                        sign=-1 if(p+pair+parity+block)%2 else 1
                        if eight:
                            bx,by=block%2*8,block//2*8;res=residual_8x8_single(1 if field else 16,4*sign,qps[p],depth,scaling8[p]);wrong=residual_8x8_single(16 if field else 1,4*sign,qps[p],depth,scaling8[p])
                        else:
                            bx=((block&1)+((block>>2)&1)*2)*4;by=(((block>>1)&1)+(block>>3)*2)*4;q=qps[p]+6*(depth-8);v=sign*scaling4[p]*[13,14,16,18,20,23][q%6];scaled=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6);r=[(v+32)>>6 for v in [scaled,scaled>>1,-(scaled>>1),-scaled]]
                            res=[r[y if field else x]for y in range(4)for x in range(4)];wrong=[r[x if field else y]for y in range(4)for x in range(4)]
                        for y in range(side):
                            for x in range(side):
                                at=(oy+(by+y)*step)*32+ox+bx+x;old=target[at];value=max(0,min(maximum,old+res[y*side+x]));scan_witness+=int(value!=max(0,min(maximum,old+wrong[y*side+x])));target[at]=value
        assert None not in target;result+=target
    return result,scan_witness

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={'qpel':0,'parity':0,'scan':0,'constrained':0}
    for(yd,cd),entropy,topology,opposite,weighted,kind in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,[False,True],[False,True],['mc','ac4','ac8','skip-bottom','mixed']):
        if topology=='frame'and opposite:continue
        for constrained in ([False,True]if kind=='mixed'else[False]):
            cabac=entropy>=0;init=max(0,entropy);depths=[yd,cd,cd];vector=(1,-3)
            source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(64)for x in range(32)]for p,depth in enumerate(depths)]
            target,count=expected(source,depths,topology,vector,kind,weighted,opposite,constrained);nearest,_=expected(source,depths,topology,vector,kind,weighted,opposite,constrained,nearest=True);changes=sum(a!=b for a,b in zip(target,nearest));assert changes>0;witness['qpel']+=changes;witness['scan']+=count
            if opposite:
                wrong,_=expected(source,depths,topology,vector,kind,weighted,False,constrained);changes=sum(a!=b for a,b in zip(target,wrong));assert changes>0;witness['parity']+=changes
            if constrained and topology!='field':
                wrong,_=expected(source,depths,topology,vector,kind,weighted,opposite,False);changes=sum(a!=b for a,b in zip(target,wrong));assert changes>0;witness['constrained']+=changes
            if kind in ['ac4','ac8','mixed']:assert count>0
            cfg=configuration(yd,cd,cabac,weighted,constrained);packets=[pcm(source,depths,topology,cabac),packet(topology,cabac,init,vector,kind,weighted,opposite),packet(topology,cabac,init,(0,0),'mc',weighted,False,index=2)]
            name=f'mbaff-p-y{yd}-c{cd}-e{entropy}-{topology}-o{int(opposite)}-w{int(weighted)}-{kind}-c{int(constrained)}'
            for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
            for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<18432H',*(sum(source,[])+target+target))) ]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,opposite=opposite,weighted=weighted,kind=kind,constrained=constrained))
    assert len(cases)==1008 and all(v>0 for v in witness.values())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
