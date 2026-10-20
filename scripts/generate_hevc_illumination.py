"""Own IC P-picture movies and independent CU boundary/pixel oracle, offline."""
import json,struct
from functools import lru_cache
from generate_hevc_multicu_refinement import FILTERS as LUMA
from generate_hevc_multicu_refinement_colored import FILTERS as CHROMA
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_cqt import OUT

def parameters(cur,ref,c):
    cur=cur[::2];ref=ref[::2];n=len(cur);shift=n.bit_length()-1
    x=sum(ref);y=sum(cur)
    if c:weight=32
    else:
        xx=sum(v*v for v in ref);xy=sum(a*b for a,b in zip(cur,ref));regular=xx>>7
        den=((xx+regular)<<shift)-x*x
        num=min(2*den,max(0,((xy+regular)<<shift)-x*y))
        ds=max(0,den.bit_length()-1-5);ns=max(0,ds-12);d=den>>ds
        reciprocal=(32768+d//2)//d if d else 0
        weight=((num>>ns)*reciprocal)>>(ds-ns+10)
    offset=(y-((weight*x)>>5)+(1<<(shift-1)))>>shift
    return weight,offset

@lru_cache(None)
def internal_plane(size,c,mx,my):
    side=size if c==0 else size//2;div=4 if c==0 else 8;left=3 if c==0 else 1
    ix,fx=divmod(mx,div);iy,fy=divmod(my,div)
    filters=LUMA if c==0 else CHROMA
    wx=[(i-left,w)for i,w in enumerate(filters[fx])if w]
    wy=[(i-left,w)for i,w in enumerate(filters[fy])if w]
    def source(x,y):return (3*x+5*y+17)%256 if c==0 else (13*x+7*y+29+83*(c-1))%256
    horizontal=[[sum(w*source(min(side-1,max(0,x+ix+dx)),y)for dx,w in wx)for x in range(side)]for y in range(side)]
    return [sum(w*horizontal[min(side-1,max(0,y+iy+dy))][x]for dy,w in wy)>>6 for y in range(side)for x in range(side)]

@lru_cache(None)
def filtered_plane(size,c,mx,my):
    return [min(255,max(0,(value+32)>>6)) for value in internal_plane(size,c,mx,my)]

def oracle(size,enabled,target_luma=(1,1,0),gains=None,mv_horizontal=0,mv_vertical=0,merge_zero_disabled=False):
    planes=[];witnesses=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        def sample(x,y,target=False):
            if c==0:
                value=(3*x+5*y+17)%256
                num,den,bias=target_luma
                return min(255,max(0,value*num//den+bias))if target else value
            return (13*x+7*y+29+83*(c-1)+([71,113][c-1]if target else 0))%256
        interpolated=filtered_plane(size,c,mv_horizontal,mv_vertical)
        out=[0]*(side*side)
        for y in range(0,side,cu):
            for x in range(0,side,cu):
                temporal=size>32 and x==0 and y==0
                mv=mv_horizontal if not temporal else 0
                dy_mv=mv_vertical if not temporal else 0
                dx=(mv+(2 if c==0 else 4))>>(2 if c==0 else 3)
                dy=(dy_mv+(2 if c==0 else 4))>>(2 if c==0 else 3)
                def reference(xx,yy):return sample(min(side-1,max(0,xx+dx)),min(side-1,max(0,yy+dy)))
                cur=[];ref=[]
                if y:
                    cur.extend(out[(y-1)*side+x+i]for i in range(cu));ref.extend(reference(x+i,y-1)for i in range(cu))
                if x:
                    cur.extend(out[(y+i)*side+x-1]for i in range(cu));ref.extend(reference(x-1,y+i)for i in range(cu))
                active=enabled and (not merge_zero_disabled or (x,y)in[(cu,0),(0,cu)])
                weight,offset=parameters(cur,ref,c) if active and not temporal and cur else (32,0)
                if c==0 and active and cur and not temporal and gains is not None:gains.append(weight)
                for yy in range(y,y+cu):
                    for xx in range(x,x+cu):
                        base=sample(xx,yy,temporal)
                        if not temporal:base=interpolated[yy*side+xx]
                        value=min(255,max(0,min(255,max(0,(base*weight)>>5))+offset))
                        out[yy*side+xx]=value;witnesses+=value!=base
        planes.extend(out)
    return planes,witnesses

def generate():
    files={};cases=[];golden=bytearray()
    for size in [32,64,128]:
        for enabled in [False,True]:
            name=f'illumination-{size}-{int(enabled)}.mp4'
            files[name]=control_movie(False,colored=True,size=size,illumination=enabled,
                merge_indices=[0]if size==32 else[2 if x==0 and y==0 else 1 if (x,y) in [(32,0),(0,32)] else 0 for y in range(0,size,32) for x in range(0,size,32)])
            values,witnesses=oracle(size,enabled);at=len(golden)//2
            golden.extend(struct.pack('<'+str(len(values))+'H',*values))
            cases.append(dict(file=name,size=size,enabled=enabled,oracle_offset=at,changed_samples=witnesses))
    files['illumination.y16']=bytes(golden)
    files['illumination.json']=(json.dumps(dict(provenance='Owned synthetic PCM/CABAC layered P movies and independent Annex I boundary regression and sample oracle. No private data, external codecs or test-time generation.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
