"""Owned spatial base-disparity DBBP, both axes and fractional prediction."""
import json,struct
from functools import lru_cache
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_dbbp_fractional import predictor
from generate_hevc_wedge_tables import OUT
MOTIONS=[(1,3),(-7,-5),(9,-11),(-64,20),(128,-256),(-128,256)]

@lru_cache(None)
def prediction(c,motion,own,size):return tuple(predictor(c,motion,own,size=size))

def oracle(size,vertical,enabled,motion,zero=False,camera=None,ignore_base=False,retain_vertical=False):
    vectors=[];output=[]
    for y in range(0,size,32):
        for x in range(0,size,32):vectors.append(dict(origin=[x,y],base=[0,0]if x==0 and y==0 or zero else list(motion)))
    def depth(x,y):return(7*min(size-1,max(0,x))+11*min(size-1,max(0,y))+37)%256
    if camera is not None:
        camera_scale,offset,precision=camera;shift=7+precision
        for record in vectors:
            ox,oy=record['origin'];base=record['base']
            dx,dy=(0,0)if ignore_base else tuple((v+2)>>2 for v in base)
            maximum=max(depth(ox+dx+x,oy+dy+y)for x,y in [(0,0),(31,0),(0,31),(31,31)])
            value=(camera_scale*maximum+offset*256+(1<<(shift-1)))>>shift
            record.update(maximum=maximum,refined=[value,base[1]if retain_vertical else 0])
    for c in range(3):
        side=size if c==0 else size//2;scale=size//side;block=32//scale;plane=[0]*(side*side)
        first=prediction(c,(0,0),True,size);second=prediction(c,tuple(motion),False,size)
        for record in vectors:
            ox,oy=record['origin'];dx,dy=[(v+2)>>2 for v in record.get('refined',record['base'])]
            bx=ox//scale;by=oy//scale
            sample=lambda x,y:depth(ox+dx+x,oy+dy+y)
            threshold=sum(sample(x,y)for x,y in [(0,0),(31,0),(0,31),(31,31)])//4
            label=lambda x,y:sample(x,y)>threshold
            combined=[]
            for y in range(block):
                for x in range(block):
                    choose=label(x*scale,y*scale)!=label(0,0)if enabled else(x if vertical else y)>=block//2
                    combined.append((second if choose else first)[(by+y)*side+bx+x])
            for y in range(block):
                for x in range(block):
                    px,py,nx,ny=(max(0,x-1),y,min(block-1,x+1),y)if vertical else(x,max(0,y-1),x,min(block-1,y+1))
                    nxt=label(min(31,x*scale+scale),y*scale)if vertical else label(x*scale,min(31,y*scale+scale))
                    value=combined[y*block+x]
                    if enabled and label(px*scale,py*scale)!=nxt:value=(combined[py*block+px]+2*value+combined[ny*block+nx])//4
                    plane[(by+y)*side+bx+x]=value
        output.extend(plane)
    return output,vectors

def generate():
    cases=[];golden=bytearray();witnesses=0;padding=0
    for size,direct in [(64,False),(128,False),(128,True)]:
        for vertical in [False,True]:
            for enabled in [False,True]:
                for motion in MOTIONS:
                    name=f'dbbp-base-{size}-{int(direct)}-{int(vertical)}-{int(enabled)}-{motion[0]}-{motion[1]}.mp4'
                    (OUT/name).write_bytes(control_movie(False,size=size,direct=direct,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_cross_mv=motion))
                    pixels,vectors=oracle(size,vertical,enabled,motion)
                    baseline,_=oracle(size,vertical,enabled,motion,zero=True)
                    witnesses+=sum(a!=b for a,b in zip(pixels,baseline))
                    for record in vectors:
                        x,y=record['origin'];dx,dy=[(v+2)>>2 for v in record['base']]
                        padding+=enabled and(x+dx<0 or y+dy<0 or x+dx+32>size or y+dy+32>size)
                    at=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                    cases.append(dict(file=name,size=size,direct=direct,vertical=vertical,enabled=enabled,motion=motion,vectors=vectors,offset=at))
    assert witnesses>0 and padding>0
    (OUT/'dbbp-base.y16').write_bytes(golden)
    (OUT/'dbbp-base.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4; cross-view PU motion seeds spatial base disparity, independent fractional interpolation and 2D padded contour oracle. Offline.',different_from_zero_base=witnesses,padded_contours=padding,cases=cases),indent=2)+'\n')
def generate_refined():
    cases=[];golden=bytearray();base_witnesses=0;vertical_witnesses=0
    motions=[(1,3),(-7,-5),(9,-11)]
    cameras=[(4,0,0),(-4,0,0),(128,0,0),(-128,0,0),(16,-3,1),(-16,3,1)]
    for size,direct in [(64,False),(128,False),(128,True)]:
        for vertical in [False,True]:
            for enabled in [False,True]:
                for motion in motions:
                    for camera in cameras:
                        scale,offset,precision=camera
                        name=f'dbbp-base-refined-{size}-{int(direct)}-{int(vertical)}-{int(enabled)}-{motion[0]}-{motion[1]}-{scale}-{offset}-{precision}.mp4'
                        (OUT/name).write_bytes(control_movie(True,camera=camera,size=size,direct=direct,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_cross_mv=motion))
                        pixels,vectors=oracle(size,vertical,enabled,motion,camera=camera)
                        wrong_base,_=oracle(size,vertical,enabled,motion,camera=camera,ignore_base=True)
                        wrong_vertical,_=oracle(size,vertical,enabled,motion,camera=camera,retain_vertical=True)
                        base_witnesses+=sum(a!=b for a,b in zip(pixels,wrong_base))
                        vertical_witnesses+=sum(a!=b for a,b in zip(pixels,wrong_vertical))
                        at=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                        cases.append(dict(file=name,size=size,direct=direct,vertical=vertical,enabled=enabled,motion=motion,camera=camera,vectors=vectors,offset=at))
    assert base_witnesses>0 and vertical_witnesses>0
    (OUT/'dbbp-base-refined.y16').write_bytes(golden)
    (OUT/'dbbp-base-refined.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4; independent spatial base, shifted corner maximum, signed camera refinement and replaced-vector contour oracle. Offline.',shifted_refinement_witnesses=base_witnesses,vertical_replacement_witnesses=vertical_witnesses,cases=cases),indent=2)+'\n')

if __name__=='__main__':
    generate()
    generate_refined()
