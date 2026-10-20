"""Coded mixed B residual, scalar sample oracle and Annex I A1/B1 disparity."""
import json,struct
from generate_hevc_illumination_b import movie,pattern
from generate_hevc_residual_b import combine,MODES
from generate_hevc_residual_b_temporal_fractional import plane,GAIN
from generate_hevc_cqt import OUT
MODES_MIXED=['both','past-source','source-future','l0','l1']
def is_iv(direction,index):return index==(1 if direction==0 else 2)
def disparity(cx,cy,refs,mvs):
    # I-280..I-282. Earlier-AU reference pictures here are all intra, so
    # temporal disparity is absent. For these 32x32 raster CUs, A1/B1 are
    # available across CTU rows; only temporal skip fallback has that restriction.
    for nx,ny in [(cx-1,cy),(cx,cy-1)]:
        if nx<0 or ny<0 or(nx,ny)==(0,0):continue
        for direction,index in enumerate(refs):
            if index is not None and is_iv(direction,index):return mvs[direction]
    return(0,0)
def oracle(size,refs,mvs,weight,weighted):
    pixels=[];witnesses=0;b1=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        def value(x,y,dv,first):
            indices=(0,0)if first else refs;vectors=((0,0),(0,0))if first else mvs;values=[];i=y*side+x
            for direction,index in enumerate(indices):
                if index is None:continue
                mv=vectors[direction]
                if not is_iv(direction,index):
                    bias=97 if direction else 0
                    base=plane(size,c,bias,True,mv)[i]
                    a=plane(size,c,0,False,dv)[i]
                    b=plane(size,c,bias,False,tuple(u+v for u,v in zip(mv,dv)))[i]
                else:
                    base=plane(size,c,0,False,mv)[i]
                    other=1-direction;other_index=indices[other]
                    z=other if other_index is not None and not is_iv(other,other_index) else 0 if other_index is not None else direction
                    temporal_mv=vectors[z]if other_index is not None and not is_iv(other,other_index)else(0,0)
                    bias=97 if z else 0
                    a=plane(size,c,bias,True,temporal_mv)[i]
                    b=plane(size,c,bias,False,tuple(u+v for u,v in zip(mv,temporal_mv)))[i]
                values.append(base+((a-b)>>(weight-1)))
            return combine(values,indices,c,weighted)
        for y in range(side):
            for x in range(side):
                cx=x//cu;cy=y//cu;first=cx==0 and cy==0
                dv=disparity(cx,cy,refs,mvs)
                actual=value(x,y,dv,first)
                changed=dv!=(0,0)and actual!=value(x,y,(0,0),first)
                witnesses+=changed;b1+=changed and cx==0 and cy>=2
                pixels.append(actual)
    return pixels,witnesses,b1

def generate():
    cases=[];golden=bytearray();witnesses=0;b1_total=0
    for size in [64,128]:
        for direct in [False,True]:
            for mode in MODES_MIXED:
                refs=MODES[mode]
                for weight in [1,2]:
                    for weighted in [False,True]:
                        for mvs in [((1,3),(-3,-1)),((-7,5),(5,-7)),((2,-3),(-1,6))]:
                            suffix='-'.join(str(v)for mv in mvs for v in mv)
                            name=f'residual-b-mixed-fractional-{size}-{int(direct)}-{mode}-{weight}-{int(weighted)}-{suffix}.mp4'
                            (OUT/name).write_bytes(movie(size,direct,False,mode,GAIN,mvs,residual_prediction=weight,weighted=weighted,references=refs,temporal_motion=True))
                            pixels,changed,b1=oracle(size,refs,mvs,weight,weighted);at=len(golden)//2;witnesses+=changed;b1_total+=b1
                            golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                            cases.append(dict(file=name,size=size,direct=direct,mode=mode,weight=weight,weighted=weighted,mvs=mvs,references=refs,oracle_offset=at,disparity_witnesses=changed,b1_witnesses=b1))
    (OUT/'residual-b-mixed-fractional.y16').write_bytes(golden)
    (OUT/'residual-b-mixed-fractional.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4; independent scalar interpolation, reference-list selection and A1/B1 disparity oracle; offline.',disparity_witnesses=witnesses,b1_witnesses=b1_total,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
