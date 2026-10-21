"""Owned full-resolution P field component-specific deblocking and chroma QP mapping."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_field_inter import configuration,pcm,field_header
from generate_avc_444_inter_deblock import filtered
from generate_avc_444_deblock import coefficient,chroma_qp
from generate_avc_pcm_formats import mux
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-field-inter-deblock'
def packet(index,bottom,cabac,init,mode,active):
    b=field_header(index,bottom,False,cabac,False,False,init,filter_mode=mode)
    if cabac:c=Cabac444Writer(init,26)
    for mb in range(2):
        if cabac:
            c.decision(11+int(mb>0),0)
            for ctx in [14,15,16]:c.decision(ctx,0)
            for axis in range(2):c.mvd(axis,0,0)
            for _ in range(4):c.decision(73,1)
            c.decision(399,0);c.decision(60,0)
        else:b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(9);b.u(0);b.se(0)
        for p in range(3):
            for block in range(16):
                sign=-1 if(p+mb)%2 else 1
                if cabac:
                    x,y=(block&1)+((block>>2)&1)*2,((block>>1)&1)+(block>>3)*2
                    inc=int(active[p]and(x>0 or mb>0))+2*int(active[p]and y>0)
                    cbf=[93,468,480][p];c.decision(cbf+inc,int(active[p]))
                    if active[p]:
                        sig=[306,805,849][p];last=[367,893,937][p];absolute=[247,972,1002][p]
                        for ctx,v in [(sig,1),(last,1),(absolute+1,1),(absolute+5,1),(absolute+5,1),(absolute+5,0)]:c.decision(ctx,v)
                        c.bypass(int(sign<0))
                elif active[p]:coefficient(b,4*sign,0)
                else:b.u(1)
        if cabac and mb==0:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={}
    for(yd,cd),entropy,reverse,activity,offsets,mode in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],[False,True],['all','cb','cr'],[(0,0),(-6,6)],[0,1,2]):
        cabac=entropy>=0;init=max(0,entropy);depths=[yd,cd,cd];active=[activity=='all',activity in ['all','cb'],activity in ['all','cr']];qps=[26,chroma_qp(26,offsets[0],cd),chroma_qp(26,offsets[1],cd)]
        source=[[(90+20*p+7*(y%2)+2*((x//4+(y//2)//4)%2))*(1<<(depth-8))for y in range(32)for x in range(32)]for p,depth in enumerate(depths)]
        order=[True,False]if reverse else[False,True];packets=[pcm(i,bottom,source,depths,cabac)for i,bottom in enumerate(order)];targets={}
        for index,bottom in enumerate(order):
            packets.append(packet(index,bottom,cabac,init,mode,active));components=[]
            for p,depth in enumerate(depths):
                raw=[v for y in range(16)for v in source[p][(y*2+int(bottom))*32:(y*2+int(bottom)+1)*32]]
                for mb in range(2):
                    if not active[p]:continue
                    sign=-1 if(p+mb)%2 else 1;q=qps[p]+6*(depth-8);v=4*sign*16*[10,11,13,14,16,18][q%6];dc=v<<(q//6-4)if q//6>=4 else(v+(1<<(3-q//6)))>>(4-q//6);delta=(dc+32)>>6
                    for y in range(16):
                        for x in range(16):at=y*32+mb*16+x;raw[at]=max(0,min((1<<depth)-1,raw[at]+delta))
                target=filtered(raw,depth,qps[p],active[p],mode);components.append(target)
                if mode!=1:
                    count=sum(a!=b for a,b in zip(raw,target));assert active[p]or count==0
                    if active[p]:key=f'{activity}-f{mode}-p{p}';witness[key]=witness.get(key,0)+count
            targets[bottom]=components
        woven=[v for p in range(3)for y in range(16)for bottom in [False,True]for v in targets[bottom][p][y*32:(y+1)*32]]
        cfg=configuration(yd,cd,cabac,False,offsets=offsets)
        name=f'field-filter-y{yd}-c{cd}-e{entropy}-r{int(reverse)}-{activity}-o{offsets[0]}-{offsets[1]}-f{mode}'
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,32,50)),('y16',struct.pack('<6144H',*(sum(source,[])+woven)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,reverse=reverse,activity=activity,offsets=offsets,mode=mode))
    assert len(cases)==432 and len(witness)==10 and all(v>0 for v in witness.values())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
