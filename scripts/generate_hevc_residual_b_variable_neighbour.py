"""Owned mixed B CUs with distinct A1/B1 motion and independent priority oracle."""
import json,struct
from generate_hevc_illumination_b import movie
from generate_hevc_residual_b import combine,MODES
from generate_hevc_residual_b_temporal_fractional import plane,GAIN
from generate_hevc_cqt import OUT
PAIRS=[((1,3),(-3,-1)),((-7,5),(5,-7))]
def current_vectors(cx,cy,a,b):return ((0,0),(0,0))if(cx,cy)==(0,0)else a if cy==0 else b
def cross_neighbour(nx,ny,refs,a,b):
    if nx<0 or ny<0 or(nx,ny)==(0,0):return None
    vectors=current_vectors(nx,ny,a,b)
    for direction,index in enumerate(refs):
        if index==(1 if direction==0 else 2):return vectors[direction]
    return None
def oracle(size,refs,a,b,weight,weighted):
    pixels=[];witness=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        def value(x,y,dv):
            cx=x//cu;cy=y//cu;first=(cx,cy)==(0,0);indices=(0,0)if first else refs
            vectors=current_vectors(cx,cy,a,b);i=y*side+x;values=[]
            for direction,index in enumerate(indices):
                mv=vectors[direction]
                if index!=(1 if direction==0 else 2):
                    bias=97 if direction else 0
                    base=plane(size,c,bias,True,mv)[i];cur=plane(size,c,0,False,dv)[i]
                    ref=plane(size,c,bias,False,tuple(u+v for u,v in zip(mv,dv)))[i]
                else:
                    z=1-direction;tmv=vectors[z];bias=97 if z else 0
                    base=plane(size,c,0,False,mv)[i];cur=plane(size,c,bias,True,tmv)[i]
                    ref=plane(size,c,bias,False,tuple(u+v for u,v in zip(mv,tmv)))[i]
                values.append(base+((cur-ref)>>(weight-1)))
            return combine(values,indices,c,weighted)
        for y in range(side):
            for x in range(side):
                cx=x//cu;cy=y//cu
                left=cross_neighbour(cx-1,cy,refs,a,b);above=cross_neighbour(cx,cy-1,refs,a,b)
                dv=left if left is not None else above if above is not None else(0,0)
                actual=value(x,y,dv)
                witness+=left is not None and above is not None and left!=above and actual!=value(x,y,above)
                pixels.append(actual)
    return pixels,witness

def generate():
    cases=[];golden=bytearray();witness=0
    for size in [64,128]:
        for direct in [False,True]:
            for mode in ['past-source','source-future']:
                for weight in [1,2]:
                    for weighted in [False,True]:
                        for swap in [0,1]:
                            a=PAIRS[swap];b=PAIRS[1-swap];refs=MODES[mode]
                            name=f'residual-b-variable-neighbour-{size}-{int(direct)}-{mode}-{weight}-{int(weighted)}-{swap}.mp4'
                            (OUT/name).write_bytes(movie(size,direct,False,mode,GAIN,a,residual_prediction=weight,weighted=weighted,references=refs,temporal_motion=True,additional_mvs=b))
                            pixels,changed=oracle(size,refs,a,b,weight,weighted);at=len(golden)//2;witness+=changed
                            golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                            cases.append(dict(file=name,size=size,direct=direct,mode=mode,weight=weight,weighted=weighted,swap=swap,mvs=a,additional_mvs=b,references=refs,oracle_offset=at,priority_witnesses=changed))
    (OUT/'residual-b-variable-neighbour.y16').write_bytes(golden)
    (OUT/'residual-b-variable-neighbour.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4, independent per-CU motion and A1-before-B1 scalar residual oracle; offline.',priority_witnesses=witness,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
