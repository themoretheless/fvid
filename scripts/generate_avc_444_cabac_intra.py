"""Owned nonseparate 4:4:4 CABAC compressed intra; full independent sample/filter oracles."""
import itertools,json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,configuration as base_configuration,mux
from generate_avc_444_deblock import chroma_qp,i16_samples
from generate_avc_separate_8x8_deblock import filter_edge_sample,residual_8x8_single
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-intra'


def configuration(yd,cd,offsets,crop=True):
    cfg=base_configuration(3,yd,cd,crop,cabac=True,level=41)
    n=int.from_bytes(cfg[6:8],'big');b=Writer();b.ue(0);b.ue(0);b.u(1);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0);b.u(1);b.u(0);b.se(offsets[1]);pps=b.nal(0x68)
    return cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps


def single(c,category,inc,level):
    cbf=[85,89,93,97,101,1012,460,464,468,1016,472,476,480,1020][category]
    c.decision(cbf+inc,int(level!=0))
    if not level:return
    sig=[105,120,134,149,152,402,484,499,513,660,528,543,557,718][category]
    last=[166,181,195,210,213,417,572,587,601,690,616,631,645,748][category]
    base=[227,237,247,257,266,426,952,962,972,708,982,992,1002,766][category]
    c.decision(sig,1);c.decision(last,1)
    c.decision(base+1,int(abs(level)>1))
    if abs(level)>1:
        assert abs(level)==4
        for v in [1,1,0]:c.decision(base+5,v)
    c.bypass(int(level<0))


def filtered(samples,depth,qp,mode,joined,eight):
    out=samples[:]
    if mode==1:return out
    for mb in range(2):
        for vertical in [True,False]:
            for edge in range(0,16,4):
                if (eight[mb]if isinstance(eight,list)else eight) and edge%8:continue
                if edge==0 and (not vertical or mb==0 or (mode==2 and not joined)):continue
                for line in range(16):
                    x=mb*16+(edge if vertical else line);y=line if vertical else edge
                    at=y*32+x;step=1 if vertical else 32
                    p=[out[at-(i+1)*step]for i in range(4)];q=[out[at+i*step]for i in range(4)]
                    a,b=filter_edge_sample(p,q,qp,depth,4 if edge==0 else 3)
                    for i in range(3):out[at-(i+1)*step]=a[i];out[at+i*step]=b[i]
    return out


def make_case(yd,cd,mode,joined,offsets,filter_mode,crop=True,modes=None):
    config=configuration(yd,cd,offsets,crop)
    planes=[[0]*512 for _ in range(3)];nals=[];qps=[26,chroma_qp(26,offsets[0],cd),chroma_qp(26,offsets[1],cd)]
    case_modes=modes or [mode]*2
    previous_eight=False;previous_i16=False;previous_pattern=0
    for mb in range(2):
        mode=case_modes[mb];eight=mode.startswith('i8');i16=mode.startswith('i16');zero=mode.endswith('zero')
        cb_only=mode.endswith('cb-only');cr_only=mode.endswith('cr-only')
        pattern=0 if zero or i16 else 15
        if not joined or mb==0:
            b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(filter_mode)
            if filter_mode!=1:b.se(0);b.se(0)
            while len(b.bits)%8:b.u(1)
            c=Cabac444Writer();coded=[{}for _ in range(3)];dc=[{}for _ in range(3)];ready=[set()for _ in range(3)];owners={}
        else:
            c.range-=2;c.renormalize() # end_of_slice_flag=0
        if i16:
            c.decision(3+int(joined and mb>0 and previous_i16),1);c.range-=2;c.renormalize()
            for ctx,val in [(6,0),(7,0),(9,1),(10,0)]:c.decision(ctx,val)
        else:
            c.decision(3+int(joined and mb>0 and previous_i16),0);c.decision(399+int(previous_eight and joined and mb>0),int(eight))
            for _ in range(4 if eight else 16):c.decision(68,1)
            for block in range(4):
                bx,by=block%2,block//2
                left=(pattern>>(block-1))&1 if bx else ((previous_pattern>>(by*2+1))&1 if joined and mb else None)
                top=(pattern>>(block-2))&1 if by else None
                c.decision(73+int(left==0)+2*int(top==0),(pattern>>block)&1)
        if i16 or pattern:c.decision(60,0)
        owners[mb]=eight
        for p,depth in enumerate([yd,cd,cd]):
            active=not zero and (not cb_only or p==1) and (not cr_only or p==2)
            sign=-1 if (mb+p)%2 else 1
            level=sign*(4 if eight else 1) if active else 0
            if i16:
                inc=2+dc[p].get(mb-1,1) if joined and mb else 3
                single(c,[0,6,10][p],inc,level);dc[p][mb]=int(active)
                values=i16_samples(depth,qps[p],level,0)
                base=(sum(planes[p][y*32+15]for y in range(16))+8)//16 if joined and mb else 1<<(depth-1)
                for y in range(16):
                    for x in range(16):planes[p][y*32+mb*16+x]=max(0,min((1<<depth)-1,base+values[y*16+x]-(1<<(depth-1))))
                for by in range(4):
                    for bx in range(4):coded[p][mb*4+bx,by]=0
                for by in range(2):
                    for bx in range(2):ready[p].add((mb*2+bx,by))
            elif eight:
                dc[p][mb]=0
                def flag(x,y):
                    if x<0 or y<0:return 1
                    owner=x//4
                    if owner not in owners:return 1
                    if not owners[owner]:return 0
                    return coded[p].get((x,y),1)
                for block in range(4):
                    x=mb*4+block%2*2;y=block//2*2
                    inc=flag(x-1,y)+2*flag(x,y-1)
                    if pattern:single(c,[5,9,13][p],inc,level)
                    for dy in range(2):
                        for dx in range(2):coded[p][x+dx,y+dy]=int(active)
                delta=residual_8x8_single(0,level,qps[p],depth,16)[0]
                for block in range(4):
                    x=mb*16+block%2*8;y=block//2*8
                    top=y>0 and (x//8,(y-1)//8)in ready[p]
                    left=x>0 and ((x-1)//8,y//8)in ready[p]
                    corner=x>0 and y>0 and ((x-1)//8,(y-1)//8)in ready[p]
                    edge=[];c0=planes[p][(y-1)*32+x-1]if corner else None
                    if top:
                        t=planes[p][(y-1)*32+x:(y-1)*32+x+8]
                        t8=planes[p][(y-1)*32+x+8]if x+8<32 and ((x+8)//8,(y-1)//8)in ready[p]else t[7]
                        edge.append(((c0 if corner else t[0])+2*t[0]+t[1]+2)//4)
                        edge.extend((t[i-1]+2*t[i]+t[i+1]+2)//4 for i in range(1,7))
                        edge.append((t[6]+2*t[7]+t8+2)//4)
                    if left:
                        l=[planes[p][(y+i)*32+x-1]for i in range(8)]
                        edge.append(((c0 if corner else l[0])+2*l[0]+l[1]+2)//4)
                        edge.extend((l[i-1]+2*l[i]+l[i+1]+2)//4 for i in range(1,7))
                        edge.append((l[6]+3*l[7]+2)//4)
                    pred=(sum(edge)+len(edge)//2)//len(edge)if edge else 1<<(depth-1)
                    value=max(0,min((1<<depth)-1,pred+delta))
                    for row in range(8):planes[p][(y+row)*32+x:(y+row)*32+x+8]=[value]*8
                    ready[p].add((x//8,y//8))
            else:
                dc[p][mb]=0
                prime=qps[p]+6*(depth-8);v=level*[10,11,13,14,16,18][prime%6]*16
                scaled=v<<(prime//6-4)if prime//6>=4 else(v+(1<<(3-prime//6)))>>(4-prime//6)
                residual=(scaled+32)>>6
                for block in range(16):
                    bx=(block//4)%2*2+block%2;by=(block//8)*2+(block%4)//2
                    x=mb*4+bx;y=by
                    inc=coded[p].get((x-1,y),1)+2*coded[p].get((x,y-1),1)
                    if pattern:single(c,[2,8,12][p],inc,level)
                    coded[p][x,y]=int(active)
                    px=x*4;py=y*4;edges=[]
                    if px and (bx or joined):edges += [planes[p][(py+j)*32+px-1]for j in range(4)]
                    if py:edges+=planes[p][(py-1)*32+px:(py-1)*32+px+4]
                    pred=(sum(edges)+len(edges)//2)//len(edges)if edges else 1<<(depth-1)
                    value=max(0,min((1<<depth)-1,pred+residual))
                    for j in range(4):planes[p][(py+j)*32+px:(py+j)*32+px+4]=[value]*4
                for by in range(2):
                    for bx in range(2):ready[p].add((mb*2+bx,by))
        previous_eight=eight;previous_i16=i16;previous_pattern=pattern
        if not joined or mb==1:
            b.bits.extend(c.finish());nal=b.nal(0x65,trailing=False);nals.append(len(nal).to_bytes(4,'big')+nal)
    expected=sum([filtered(plane,depth,qps[p],filter_mode,joined,[m.startswith('i8')for m in case_modes])for p,(plane,depth)in enumerate(zip(planes,[yd,cd,cd]))],[])
    return config,b''.join(nals),expected


def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),mode,offsets,filter_mode in itertools.product([(8,8),(10,12),(14,8)],['i16-zero','i16-dc','i4-zero','i4-dc','i4-cb-only','i8-zero','i8-dc','i8-cb-only','i8-cr-only'],[(0,0),(-6,6),(6,-6)],[0,1,2]):
        for joined in [False,True]:
            name=f'{mode}-y{yd}-c{cd}-o{offsets[0]}-{offsets[1]}-f{filter_mode}-j{int(joined)}'
            cfg,packet,expected=make_case(yd,cd,mode,joined,offsets,filter_mode)
            for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,16,25)),('y16',struct.pack('<1536H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(name=name,yd=yd,cd=cd,crop=[1]*4))
    assert len(cases)==486
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n')
    print('CABAC cases',len(cases))
if __name__=='__main__':generate()
