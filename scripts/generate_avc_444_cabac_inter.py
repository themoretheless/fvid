"""Owned full 444 P CABAC, independent component CBFs, PCM restart and skip/intra contexts."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_inter import configuration,make_case,Writer,mux,WEIGHTS
from generate_avc_444_cabac_intra import single
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-inter'
def header(index,weighted,mode=1,poc=None,init_idc=0):
    b=Writer();b.ue(0);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
    if index==0:b.ue(0)
    b.u(index*2 if poc is None else poc,4)
    if index==0:b.u(0);b.u(0)
    else:
        b.u(0);b.u(0)
        if weighted:
            b.ue(1 if index==1 else 0);b.ue(1 if index==1 else 0);b.u(int(index==1))
            if index==1:b.se(WEIGHTS[0][0]);b.se(WEIGHTS[0][1])
            b.u(int(index==1))
            if index==1:
                for weight,offset in WEIGHTS[1:]:b.se(weight);b.se(offset)
        b.u(0);b.ue(init_idc)
    b.se(0);b.ue(mode)
    if mode!=1:b.se(0);b.se(0)
    while len(b.bits)%8:b.u(1)
    return b

def pcm(source,depths):
    b=header(0,False);c=Cabac444Writer()
    for mb in range(2):
        c.decision(3+int(mb>0),1);b.bits.extend(c.finish());b.align()
        for p,depth in enumerate(depths):
            for y in range(16):
                for x in range(16):b.u(source[p][y*32+mb*16+x],depth)
        c.low=0;c.range=510;c.width=9
        if mb==0:c.range-=2;c.renormalize()
    b.bits.extend(c.finish());nal=b.nal(0x65,trailing=False);return len(nal).to_bytes(4,'big')+nal

def p_packet(vector,kind,weighted,active=(True,True,True),mode=1,init_idc=0):
    b=header(1,weighted,mode,init_idc=init_idc);c=Cabac444Writer(init_idc,26)
    for mb in range(2):
        skip=kind=='skip' or(kind=='skip-intra'and mb==0)
        c.decision(11+int(mb>0 and kind not in ['skip','skip-intra']),int(skip))
        if not skip:
            intra=kind in ['mixed','skip-intra']and mb==1
            c.decision(14,int(intra))
            if intra:
                c.decision(17,1);c.range-=2;c.renormalize()
                for ctx,v in [(18,0),(19,0),(20,1),(20,0),(60,0)]:c.decision(ctx,v)
                # Left is inter/skip with zero DC CBF; top is intra-unavailable.
                for category in [0,6,10]:single(c,category,2,0)
            else:
                c.decision(15,0);c.decision(16,0)
                for axis in range(2):c.mvd(axis,vector[axis]if mb==0 else 0,abs(vector[axis])if mb else 0)
                residual=kind in ['res4','res8','mixed'];eight=kind=='res8'
                for ctx in ([73]*4 if residual else[73,74,75,76]if mb==0 else[74,74,76,76]):c.decision(ctx,int(residual))
                if residual:
                    c.decision(399+int(eight and mb>0),int(eight));c.decision(60,0)
                    for p in range(3):
                        for block in range(4 if eight else 16):
                            if eight:x,y=block%2,block//2
                            else:x,y=(block&1)+((block>>2)&1)*2,((block>>1)&1)+(block>>3)*2
                            inc=int(active[p]and(x>0 or mb>0))+2*int(active[p]and y>0)
                            single(c,([5,9,13]if eight else[2,8,12])[p],inc,(4 if(p+mb)%2==0 else-4)if active[p]else 0)
        if mb==0:c.range-=2;c.renormalize()
    b.bits.extend(c.finish());nal=b.nal(0x41,trailing=False);return len(nal).to_bytes(4,'big')+nal

def echo(weighted,poc=None,init_idc=0):
    b=header(2,weighted,poc=poc,init_idc=init_idc);c=Cabac444Writer(init_idc,26);c.decision(11,1);c.range-=2;c.renormalize();c.decision(11,1);b.bits.extend(c.finish())
    nal=b.nal(0x41,trailing=False);return len(nal).to_bytes(4,'big')+nal

def make(yd,cd,vector,kind,weighted,init_idc=0):
    if kind!='skip-intra':_,_,expected=make_case(yd,cd,vector,kind,weighted)
    else:
        _,_,words=make_case(yd,cd,(0,0),'skip',weighted);source=words[:1536];target=words[1536:3072]
        for p in range(3):
            dc=(sum(target[p*512+y*32+15]for y in range(16))+8)//16
            for y in range(16):target[p*512+y*32+16:p*512+y*32+32]=[dc]*16
        expected=source+target+target
    source=[expected[p*512:(p+1)*512]for p in range(3)]
    return configuration(yd,cd,weighted,cabac=True),[pcm(source,[yd,cd,cd]),p_packet(vector,kind,weighted,init_idc=init_idc),echo(weighted,init_idc=init_idc)],expected

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for(yd,cd),vector,kind,weighted,init_idc in itertools.product([(8,8),(10,12),(14,8)],[(0,0),(-3,2),(8,-4)],['skip','mc','res4','res8','mixed','skip-intra'],[False,True],[0,1,2]):
        name=f'cabac-p-y{yd}-c{cd}-x{vector[0]}-y{vector[1]}-{kind}-w{int(weighted)}-i{init_idc}'
        cfg,packets,expected=make(yd,cd,vector,kind,weighted,init_idc)
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,16,25)),('y16',struct.pack('<4608H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,kind=kind,weighted=weighted,vector=vector,init_idc=init_idc))
    assert len(cases)==324
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
