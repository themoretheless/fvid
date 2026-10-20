"""Offline coded residual motion and independent scalar 2-tap sample oracle."""
import json,struct
from functools import lru_cache
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
from generate_hevc_illumination import internal_plane

@lru_cache(None)
def sample_planes(size,c,bias,mx,my):
    side=size if c==0 else size//2;den=4 if c==0 else 8
    dx,fx=divmod(mx,den);dy,fy=divmod(my,den)
    def sample(x,y,kind):
        x=min(side-1,max(0,x));y=min(side-1,max(0,y))
        value=(3*x+5*y+17)%256 if c==0 else (13*x+7*y+29+83*(c-1))%256
        if kind==2:return value//2+30 if c==0 else(value+[71,113][c-1])%256
        return min(255,max(0,value+bias))if kind==1 and c==0 else value
    def interp(x,y,kind):
        a=sample(x+dx,y+dy,kind)*(den-fx)+sample(x+dx+1,y+dy,kind)*fx
        b=sample(x+dx,y+dy+1,kind)*(den-fx)+sample(x+dx+1,y+dy+1,kind)*fx
        return (a*(den-fy)+b*fy)*64//(den*den)
    return tuple([interp(x,y,kind)for y in range(side)for x in range(side)]for kind in range(3))

def generate():
    cases=[];golden=bytearray();different=0;half_precision=0
    mvs=[(x,(3*x)%8)for x in range(1,8)]+[(-7,-5),(-1,3),(0,-7),(0,5)]
    for size in [64,128]:
        for direct in [False,True]:
            for weight in [1,2]:
                for bias in [-80,20,200]:
                    for mx,my in mvs:
                        for reference in [0,1]:
                            name=f'residual-fractional-{size}-{int(direct)}-{weight}-{bias}-{mx}-{my}{"-temporal"if reference==0 else""}.mp4'
                            indices=[2 if(x,y)==(0,0)else 1 if(x,y)in[(32,0),(0,32)]else 0 for y in range(0,size,32)for x in range(0,size,32)]
                            (OUT/name).write_bytes(control_movie(False,colored=True,size=size,direct=direct,
                                inter_view=False,residual_prediction=weight,source_luma=(1,1,bias),target_luma=(1,2,30),ic_mv=(mx,my),merge_indices=indices,ic_reference=reference))
                            at=len(golden)//2;pixels=[]
                            for c in range(3):
                                side=size if c==0 else size//2;cu=32 if c==0 else 16
                                past,current,own_interpolated=sample_planes(size,c,bias,mx,my)
                                eight=internal_plane(size,c,mx,my)
                                different+=sum(a!=b for i,(a,b)in enumerate(zip(past,eight))if i%side>=cu or i//side>=cu)
                                for y in range(side):
                                    for x in range(side):
                                        source=(3*x+5*y+17)%256 if c==0 else(13*x+7*y+29+83*(c-1))%256
                                        own=source//2+30 if c==0 else(source+[71,113][c-1])%256
                                        cur=min(255,max(0,source+bias))if c==0 else source
                                        if x<cu and y<cu:
                                            base,a,b=own*64,cur*64,source*64
                                        else:
                                            i=y*side+x
                                            base,a,b=(current[i],own*64,past[i])if reference==1 else(own_interpolated[i],cur*64,past[i])
                                        raw=base+((a-b)>>(weight-1))
                                        if weight==2:
                                            early=((base+32)>>6)+((((a+32)>>6)-((b+32)>>6))>>1)
                                            half_precision+=min(255,max(0,(raw+32)>>6))!=min(255,max(0,early))
                                        pixels.append(min(255,max(0,(raw+32)>>6)))
                            golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                            cases.append(dict(file=name,size=size,direct=direct,weight=weight,bias=bias,mv=[mx,my],reference=reference,oracle_offset=at))
    (OUT/'residual-fractional.y16').write_bytes(golden)
    (OUT/'residual-fractional.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4, independent scalar bilinear oracle; offline, no external codec or private data.',different_from_eight_tap=different,half_precision_witnesses=half_precision,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
