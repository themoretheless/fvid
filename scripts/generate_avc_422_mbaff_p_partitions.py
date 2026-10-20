"""Owned MBAFF P partition syntax, independent motion grids and 422 sampling."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p-partitions'
MODES_P=['16x8','8x16','8x8','8x4','4x8','4x4','mixed','ref0-8x8']
def partitions(mode):
    if mode=='16x8':return 1,None,[(0,0,16,8,0),(0,8,16,8,1)]
    if mode=='8x16':return 2,None,[(0,0,8,16,0),(8,0,8,16,1)]
    codes=[0,1,2,3] if mode=='mixed' else [{'8x8':0,'8x4':1,'4x8':2,'4x4':3,'ref0-8x8':0}[mode]]*4
    parts=[]
    for group,code in enumerate(codes):
        w,h=[(8,8),(8,4),(4,8),(4,4)][code]
        for y in range(0,8,h):
            for x in range(0,8,w):parts.append((group%2*8+x,group//2*8+y,w,h,group))
    return 4 if mode=='ref0-8x8' else 3,codes,parts

def predictor(candidates,kind):
    a,b,c,d=candidates;c=d if c is None else c
    preferred=b if kind=='top' else a if kind in ['bottom','left'] else c if kind=='right' else None
    if preferred is not None:return preferred
    if b is None and c is None and a is not None:b=c=a
    available=[v for v in [a,b,c] if v is not None]
    if len(available)==1:return available[0]
    return tuple(sorted((a or (0,0),b or (0,0),c or (0,0))[i][axis] for i in range(3))[1] for axis in range(2))

def state(topology,mode,negative):
    _,_,parts=partitions(mode);vectors={};differences={};magnitudes={};refmaps={};conditions={}
    for pair,field in enumerate(MODES[topology]):
        # One pair per slice: only this dictionary's decoded cells are available.
        cells={}
        for parity in range(2):
            ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1;address=pair*2+parity
            for i,(x,y,w,h,group) in enumerate(parts):
                absolute=[(ox+x-1,oy+y*step),(ox+x,oy+(y-1)*step),(ox+x+w,oy+(y-1)*step),(ox+x-1,oy+(y-1)*step)]
                candidates=[cells.get(at) for at in absolute]
                kind=('top' if y==0 else 'bottom') if mode=='16x8' else ('left' if x==0 else 'right') if mode=='8x16' else 'median'
                pred=predictor(candidates,kind);sign=-1 if (pair+parity+i+int(negative))%2 else 1;delta=(sign*(1+i%2),-sign*(1+(i//2)%2));vector=tuple(a+b for a,b in zip(pred,delta));differences[(address,i)]=delta
                for row in range(y,y+h):
                    for col in range(x,x+w):cells[(ox+col,oy+row*step)]=vector;vectors[(address,col//4,row//4)]=vector
    assert len(vectors)==128 and len(set(vectors.values()))>1
    return vectors,differences

def packet(cabac,topology,aso,opposite,mode,negative,qp=26,filter_mode=1,luma_residual=False):
    code,subcodes,parts=partitions(mode);vectors,differences=state(topology,mode,negative);nals=[]
    for pair,field in enumerate(MODES[topology]):
        b=Writer();b.ue(pair);b.ue(0);b.ue(0);b.u(1,4);b.u(0);b.u(2,4);b.u(0);b.u(0);b.u(0)
        if cabac:b.ue(0)
        b.se(qp-26);b.ue(filter_mode)
        if filter_mode!=1:b.se(0);b.se(0)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,qp)
        refs={};mags={}
        for parity in range(2):
            address=pair*2+parity;ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1;bottom=parity==1 and not field
            if not cabac:
                b.ue(0)
                if parity==0:b.u(int(field))
                b.ue(code)
                if subcodes is not None:
                    for v in subcodes:b.ue(v)
            else:
                assert code!=4
                c.decision(11+int(bottom),0)
                if parity==0:c.decision(70,int(field))
                c.decision(14,0);c.decision(15,int(code in [1,2]));c.decision(17 if code in [1,2] else 16,int(code in [1,3]))
                if subcodes is not None:
                    for v in subcodes:
                        for i,symbol in enumerate(['1','00','011','010'][v]):c.decision(21+i,int(symbol))
            for group in sorted({p[4] for p in parts}):
                group_parts=[p for p in parts if p[4]==group];x,y,_,_,_=group_parts[0]
                reference=int(field and opposite)
                if field and code!=4:
                    if not cabac:b.u(int(not opposite))
                    else:
                        a=refs.get((ox+x-1,oy+y*step),0);above=refs.get((ox+x,oy+(y-1)*step),0)
                        c.decision(54+int(a>0)+2*int(above>0),reference)
                        if reference:c.decision(58,0)
                for x,y,w,h,_ in group_parts:
                    for row in range(y,y+h):
                        for col in range(x,x+w):refs[(ox+col,oy+row*step)]=reference
            for i,(x,y,w,h,_) in enumerate(parts):
                delta=differences[(address,i)]
                if not cabac:
                    for v in delta:b.se(v)
                else:
                    a=mags.get((ox+x-1,oy+y*step),(0,0));above=mags.get((ox+x,oy+(y-1)*step),(0,0))
                    for axis,v in enumerate(delta):c.mvd(axis,v,a[axis]+above[axis])
                for row in range(y,y+h):
                    for col in range(x,x+w):mags[(ox+col,oy+row*step)]=tuple(abs(v) for v in delta)
            if not cabac:
                b.ue(11 if luma_residual else 0)
                if luma_residual:
                    b.se(0)
                    for block in range(16):
                        bx=block//4%2*2+block%2;by=block//8*2+block//2%2
                        b.u(1,2);b.u((pair+parity+bx+by+int(negative))%2);b.u(1)
            else:
                for ctx in ([73]*4 if luma_residual else [75,76,75,76] if bottom else [73,74,75,76]):c.decision(ctx,int(luma_residual))
                c.decision(77,0)
                if luma_residual:
                    c.decision(60,0)
                    for block in range(16):
                        bx=block//4%2*2+block%2;by=block//8*2+block//2%2
                        c.decision(93+int(bx>0)+2*int(by>0 or bottom),1)
                        c.decision(306 if field else 134,1);c.decision(367 if field else 195,1);c.decision(248,0)
                        c.bypass((pair+parity+bx+by+int(negative))%2)
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x41,trailing=not cabac))
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

@lru_cache(None)
def samples(yd,cd,topology,opposite,mode,negative):
    _,source=picture(2,yd,cd,False,topology,False,0);vectors,_=state(topology,mode,negative)
    target=expected(source,yd,cd,topology,(0,0),opposite,cell_motions=vectors)
    assert target!=expected(source,yd,cd,topology,vectors[(0,0,0)],opposite),'partition variation must change clipped pixels'
    return source,target

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,opposite,mode,negative in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],MODES_P,[False,True]):
        if topology=='frame' and opposite:continue
        if mode=='ref0-8x8' and (cabac or opposite):continue
        aso=negative^cabac;crop=negative or opposite;name=f'422-mbaff-parts-y{yd}-c{cd}-e{int(cabac)}-{topology}-opp{int(opposite)}-{mode}-n{int(negative)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True);source,target=samples(yd,cd,topology,opposite,mode,negative);first,original=picture(2,yd,cd,cabac,topology,aso,0);assert source==original
        packets=[first,packet(cabac,topology,aso,opposite,mode,negative),prediction(cabac,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,opposite=opposite,mode=mode,negative=negative,aso=aso,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==612
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('612 own MBAFF P partition movies')
if __name__=='__main__':generate()
