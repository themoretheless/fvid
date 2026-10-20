"""Owned B residual temporal MV scaling, independently sampled at Q6 precision."""
import json,struct
from functools import lru_cache
from generate_hevc_illumination_b import movie,pattern
from generate_hevc_residual_b import combine,MODES
from generate_hevc_cqt import OUT
GAIN=(2,1,-40)
@lru_cache(None)
def plane(size,c,bias,target,mv):
    side=size if c==0 else size//2;den=4 if c==0 else 8
    dx,fx=divmod(mv[0],den);dy,fy=divmod(mv[1],den)
    def sample(x,y):return pattern(min(side-1,max(0,x)),min(side-1,max(0,y)),c,target,bias,GAIN)
    return [((sample(x+dx,y+dy)*(den-fx)+sample(x+dx+1,y+dy)*fx)*(den-fy)
        +(sample(x+dx,y+dy+1)*(den-fx)+sample(x+dx+1,y+dy+1)*fx)*fy)*64//(den*den)
        for y in range(side)for x in range(side)]
def oracle(size,refs,mvs,weight,weighted):
    pixels=[];witness=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        filtered=[];unscaled=[]
        for direction,index in enumerate(refs):
            target=0 if direction==0 else 2
            original=2 if(direction==0 and index==2)or(direction==1 and index==0)else 0
            mv=tuple(v*(1 if original==target else-1)for v in mvs[direction])
            bias=97 if direction else 0
            filtered.append((plane(size,c,bias,True,mv),plane(size,c,bias,False,mv)))
            unscaled.append((plane(size,c,bias,True,mvs[direction]),plane(size,c,bias,False,mvs[direction])))
        for y in range(side):
            for x in range(side):
                first=x<cu and y<cu;values=[];wrong=[]
                cur=pattern(x,y,c,False,0,GAIN)*64
                for direction in range(2):
                    bias=97 if direction else 0
                    if first:
                        a=pattern(x,y,c,True,bias,GAIN)*64;b=pattern(x,y,c,False,bias,GAIN)*64
                        raw=a+((cur-b)>>(weight-1));values.append(raw);wrong.append(raw)
                    else:
                        a,b=filtered[direction];aa,bb=unscaled[direction];i=y*side+x
                        values.append(a[i]+((cur-b[i])>>(weight-1)))
                        wrong.append(aa[i]+((cur-bb[i])>>(weight-1)))
                indices=(0,0)if first else refs
                value=combine(values,indices,c,weighted)
                witness+=value!=combine(wrong,indices,c,weighted)
                pixels.append(value)
    return pixels,witness

def generate():
    cases=[];golden=bytearray();scaled=0
    for size in [64,128]:
        for direct in [False,True]:
            for mode in ['temporal','future-past']:
                refs=MODES[mode]
                for weight in [1,2]:
                    for weighted in [False,True]:
                        for mvs in [((1,3),(-3,-1)),((-7,5),(5,-7)),((2,-3),(-1,6))]:
                            suffix='-'.join(str(v)for mv in mvs for v in mv)
                            name=f'residual-b-temporal-fractional-{size}-{int(direct)}-{mode}-{weight}-{int(weighted)}-{suffix}.mp4'
                            (OUT/name).write_bytes(movie(size,direct,False,mode,GAIN,mvs,residual_prediction=weight,weighted=weighted,references=refs,temporal_motion=True))
                            pixels,witness=oracle(size,refs,mvs,weight,weighted);at=len(golden)//2;scaled+=witness
                            golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                            cases.append(dict(file=name,size=size,direct=direct,mode=mode,weight=weight,weighted=weighted,mvs=mvs,references=refs,oracle_offset=at,scaling_witnesses=witness))
    (OUT/'residual-b-temporal-fractional.y16').write_bytes(golden)
    (OUT/'residual-b-temporal-fractional.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4 and scalar bilinear/POC-sign oracle. Offline, no external codecs.',scaling_witnesses=scaled,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
