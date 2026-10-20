"""Owned depth-refined DBBP contours, signed camera and padding oracle, offline."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT

CAMERAS=[(4,0,0),(-4,0,0),(128,0,0),(-128,0,0),(1,0,0),(-1,0,0),(16,-3,1),(-16,3,1)]

def oracle(size,cu,vertical,enabled,camera,zero=False):
    scale,offset,precision=camera;shift=7+precision
    def depth(x,y):return (7*min(size-1,max(0,x))+11*min(size-1,max(0,y))+37)%256
    output=[];vectors=[]
    for oy in range(0,size,cu):
        for ox in range(0,size,cu):
            maximum=max(depth(ox+x,oy+y)for x,y in [(0,0),(cu-1,0),(0,cu-1),(cu-1,cu-1)])
            vector=(scale*maximum+offset*256+(1<<(shift-1)))>>shift
            dx=0 if zero else(vector+2)>>2
            vectors.append(dict(origin=[ox,oy],maximum=maximum,vector=vector,depth_offset=dx))
    for c in range(3):
        step=1 if c==0 else 2;side=size//step;block=cu//step;plane=[0]*(side*side)
        for record in vectors:
            ox,oy=record['origin'];dx=record['depth_offset'];bx=ox//step;by=oy//step
            sample=lambda x,y:depth(ox+dx+x,oy+y)
            threshold=sum(sample(x,y)for x,y in [(0,0),(cu-1,0),(0,cu-1),(cu-1,cu-1)])//4
            label=lambda x,y:sample(x,y)>threshold
            combined=[]
            for y in range(block):
                for x in range(block):
                    second=label(x*step,y*step)!=label(0,0)if enabled else(x if vertical else y)>=block//2
                    xx=bx+x;yy=by+y
                    base=(3*xx+5*yy+17)%256 if c==0 else(13*xx+7*yy+29+83*(c-1))%256
                    value=base if second else min(255,max(0,2*base-40))if c==0 else(base+([71,113][c-1]))%256
                    combined.append(value)
            for y in range(block):
                for x in range(block):
                    px,py,nx,ny=(max(0,x-1),y,min(block-1,x+1),y)if vertical else(x,max(0,y-1),x,min(block-1,y+1))
                    nxt=label(min(cu-1,x*step+step),y*step)if vertical else label(x*step,min(cu-1,y*step+step))
                    value=combined[y*block+x]
                    if enabled and label(px*step,py*step)!=nxt:value=(combined[py*block+px]+2*value+combined[ny*block+nx])//4
                    plane[(by+y)*side+bx+x]=value
        output.extend(plane)
    return output,vectors

def generate():
    cases=[];golden=bytearray();different=0;padding=0;negative=0
    configurations=[(cu,max(32,cu),False)for cu in [16,32,64]]+[(32,128,d)for d in [False,True]]
    for cu,size,direct in configurations:
        for vertical in [False,True]:
            for enabled in [False,True]:
                for camera in CAMERAS:
                    scale,offset,precision=camera
                    name=f'dbbp-disparity-{cu}-{size}-{int(direct)}-{int(vertical)}-{int(enabled)}-{scale}-{offset}-{precision}.mp4'
                    (OUT/name).write_bytes(control_movie(True,camera=camera,size=size,direct=direct,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_log=6 if cu==64 else 5,dbbp_cu_log=cu.bit_length()-1))
                    pixels,vectors=oracle(size,cu,vertical,enabled,camera)
                    baseline,_=oracle(size,cu,vertical,enabled,camera,zero=True)
                    different+=sum(a!=b for a,b in zip(pixels,baseline))
                    padding+=enabled*sum(record['origin'][0]+record['depth_offset']<0 or record['origin'][0]+record['depth_offset']+cu>size for record in vectors)
                    negative+=enabled*sum(record['depth_offset']<0 for record in vectors)
                    at=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                    cases.append(dict(file=name,cu=cu,size=size,direct=direct,vertical=vertical,enabled=enabled,camera=camera,vectors=vectors,offset=at))
    assert different>0 and padding>0 and negative>0
    (OUT/'dbbp-disparity.y16').write_bytes(golden)
    (OUT/'dbbp-disparity.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4, depth-reference plus DBBP SPS tools and signed camera parameters; independent corner-maximum/refinement/padded-depth contour scalar oracle. Offline.',different_from_zero_disparity=different,padded_blocks=padding,negative_offsets=negative,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
