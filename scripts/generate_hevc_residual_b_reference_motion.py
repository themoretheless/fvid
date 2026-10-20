"""Coded inter source AMP motion; own scalar source reconstruction and RP lookup."""
import json,struct
from functools import lru_cache
from generate_hevc_illumination_b import movie,pattern
from generate_hevc_cqt import texture_amp_bi,OUT
from generate_hevc_residual_b import combine,MODES
from generate_hevc_residual_b_temporal_fractional import plane,GAIN
@lru_cache(None)
def current_source(size,c,cut):
    side=size if c==0 else size//2;den=4 if c==0 else 8
    def sample(x,y,bias):return pattern(x,min(side-1,max(0,y)),c,False,bias,GAIN)
    def filtered(x,y,mv,bias):
        dy,fy=divmod(mv,den)
        if fy==0:return sample(x,y+dy,bias)*64
        assert c!=0 and fy==4
        return sum(w*sample(x,y+dy+k-1,bias)for k,w in enumerate([-4,36,36,-4]))
    pixels=[]
    for y in range(side):
        for x in range(side):
            sign=1 if((x if c==0 else x*2)%32)<cut else-1
            raw=filtered(x,y,4*sign,0)+filtered(x,y,-8*sign,97)
            pixels.append(min(255,max(0,(raw+64)>>7)))
    return pixels
@lru_cache(None)
def filtered_current(size,c,cut,mv):
    data=current_source(size,c,cut);side=size if c==0 else size//2;den=4 if c==0 else 8
    dx,fx=divmod(mv[0],den);dy,fy=divmod(mv[1],den)
    def at(x,y):return data[min(side-1,max(0,y))*side+min(side-1,max(0,x))]
    return [((at(x+dx,y+dy)*(den-fx)+at(x+dx+1,y+dy)*fx)*(den-fy)
        +(at(x+dx,y+dy+1)*(den-fx)+at(x+dx+1,y+dy+1)*fx)*fy)*64//(den*den)
        for y in range(side)for x in range(side)]
def source_lookup(size,cut,cx,cy,mv):
    # I-265..I-268: round quarter units, align before clipping, then read
    # actual AMP partition at that point. The coded source has L0 and L1.
    x=max(0,min(size-1,((cx*32+16+((mv[0]+2)>>2))>>3)<<3))
    sign=1 if x%32<cut else-1
    return sign

def oracle(size,mode,mvs,cut,weight,weighted):
    refs=MODES[mode];result=[];zero_witness=0;l0_witness=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        def value(x,y,which):
            first=x<cu and y<cu;indices=(0,0)if first else refs;vectors=((0,0),(0,0))if first else mvs
            i=y*side+x;values=[]
            if not first:
                lookup=0 if all(v is not None for v in refs)else next(k for k,v in enumerate(refs)if v is not None)
                sign=source_lookup(size,cut,x//cu,y//cu,vectors[lookup])
            for direction,index in enumerate(indices):
                if index is None:continue
                if first:
                    bias=97 if direction else 0
                    base=plane(size,c,bias,True,(0,0))[i];a=current_source(size,c,cut)[i]*64;b=plane(size,c,bias,False,(0,0))[i]
                else:
                    z=1 if which=='l1' or (which=='zero' and mode=='l1') else 0
                    tmv=(0,0)if which=='zero'else(0,(-8 if z else 4)*sign)
                    bias=97 if z else 0;mv=vectors[direction]
                    base=filtered_current(size,c,cut,mv)[i]
                    a=plane(size,c,bias,True,tmv)[i]
                    b=plane(size,c,bias,False,tuple(u+v for u,v in zip(mv,tmv)))[i]
                values.append(base+((a-b)>>(weight-1)))
            return combine(values,indices,c,weighted)
        for y in range(side):
            for x in range(side):
                actual=value(x,y,'l0');zero_witness+=actual!=value(x,y,'zero');l0_witness+=actual!=value(x,y,'l1');result.append(actual)
    return result,zero_witness,l0_witness

def generate():
    cases=[];golden=bytearray();zero=0;priority=0
    for size in [64,128]:
        for direct in [False,True]:
            for cut in [8,24]:
                source=texture_amp_bi(size,top=cut,vertical_partition=True)
                for mode in ['both','l0','l1']:
                    for weight in [1,2]:
                        for weighted in [False,True]:
                            for mvs in [((29,3),(-35,-1)),((30,3),(-34,-1)),((135,-3),(-135,5))]:
                                suffix='-'.join(str(v)for mv in mvs for v in mv)
                                name=f'residual-b-reference-motion-{size}-{int(direct)}-{cut}-{mode}-{weight}-{int(weighted)}-{suffix}.mp4'
                                (OUT/name).write_bytes(movie(size,direct,False,mode,GAIN,mvs,residual_prediction=weight,weighted=weighted,references=MODES[mode],temporal_motion=True,inter_view=False,source_data=source,source_kind=0))
                                src_at=len(golden)//2;src=[]
                                for c in range(3):src.extend(current_source(size,c,cut))
                                golden.extend(struct.pack('<'+str(len(src))+'H',*src))
                                pixels,a,b=oracle(size,mode,mvs,cut,weight,weighted);at=len(golden)//2;zero+=a;priority+=b
                                golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                                cases.append(dict(file=name,size=size,direct=direct,mode=mode,cut=cut,weight=weight,weighted=weighted,mvs=mvs,references=MODES[mode],source_oracle_offset=src_at,oracle_offset=at,zero_witnesses=a,l0_witnesses=b))
    (OUT/'residual-b-reference-motion.y16').write_bytes(golden)
    (OUT/'residual-b-reference-motion.json').write_text(json.dumps(dict(provenance='Owned AMP B source and coded residual B targets; independent standard source interpolation, aligned source-motion lookup and bilinear residual oracle; offline.',zero_witnesses=zero,l0_witnesses=priority,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
