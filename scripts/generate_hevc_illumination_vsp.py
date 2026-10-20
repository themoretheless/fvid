"""Own within-picture VSP / explicit IC / VSP transition, independent oracle."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import parameters,filtered_plane,LUMA,CHROMA
from generate_hevc_cqt import OUT
IC_MV=(3,-3)

def oracle(size,scale):
    motion=[(0,0)]*(size*size);blocks=[]
    def depth(x,y):return(7*min(size-1,max(0,x))+11*min(size-1,max(0,y))+37)%256
    for y in range(0,size,32):
        for x in range(0,size,32):
            ic=(x,y)==(32,0)
            base=motion[(y+31)*size+x-1]if x else motion[(y-1)*size+x+31]if y else(0,0)
            if ic:
                for yy in range(y,y+32):
                    for xx in range(x,x+32):motion[yy*size+xx]=IC_MV
                blocks.append(dict(origin=[x,y],ic=True,base=base));continue
            dx,dy=[(v+2)>>2 for v in base]
            corners=[depth(x+dx,y+dy),depth(x+dx+31,y+dy),depth(x+dx,y+dy+31),depth(x+dx+31,y+dy+31)]
            horizontal=(corners[0]<corners[3])==(corners[1]<corners[2]);sw,sh=(8,4)if horizontal else(4,8)
            for sy in range(0,32,sh):
                for sx in range(0,32,sw):
                    d=max(depth(x+dx+sx+xx,y+dy+sy+yy)for yy in[0,sh-1]for xx in[0,sw-1])
                    mv=(scale*d+64)>>7
                    for yy in range(y+sy,y+sy+sh):
                        for xx in range(x+sx,x+sx+sw):motion[yy*size+xx]=(mv,0)
            blocks.append(dict(origin=[x,y],ic=False,base=base,subblock=[sw,sh]))
    planes=[];changed=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        divisor=4 if c==0 else 8;left=3 if c==0 else 1;filters=LUMA if c==0 else CHROMA
        source=lambda x,y:(3*x+5*y+17)%256 if c==0 else(13*x+7*y+29+83*(c-1))%256
        out=[0]*(side*side);ic_pixels=filtered_plane(size,c,*IC_MV)
        for y in range(0,side,cu):
            for x in range(0,side,cu):
                ic=(x,y)==(cu,0);weight,offset=32,0
                if ic:
                    shift=2 if c==0 else 3;dx,dy=[(v+(1<<(shift-1)))>>shift for v in IC_MV]
                    cur=[out[(y+i)*side+x-1]for i in range(cu)]
                    ref=[source(min(side-1,max(0,x-1+dx)),min(side-1,max(0,y+i+dy)))for i in range(cu)]
                    weight,offset=parameters(cur,ref,c)
                for yy in range(y,y+cu):
                    for xx in range(x,x+cu):
                        if ic:
                            base=ic_pixels[yy*side+xx]
                            value=min(255,max(0,min(255,(base*weight)>>5)+offset));changed+=value!=base
                        else:
                            mv=motion[(yy if c==0 else 2*yy)*size+(xx if c==0 else 2*xx)][0]
                            integer,phase=divmod(mv,divisor)
                            value=min(255,max(0,(sum(w*source(min(side-1,max(0,xx+integer+i-left)),yy)for i,w in enumerate(filters[phase]))+32)>>6))
                        out[yy*side+xx]=value
        planes.extend(out)
    return planes,motion,blocks,changed

def generate():
    files={};cases=[];golden=bytearray();vectors=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for refine in [False,True]:
                for scale in [4,-4,128,-128]:
                    name=f'illumination-vsp-{size}-{int(direct)}-{int(refine)}-{scale}.mp4'
                    flags=[(x,y)==(32,0)for y in range(0,size,32)for x in range(0,size,32)]
                    indices=[1 if x==0 and y>0 or (x,y)==(64,0) else 0 for y in range(0,size,32)for x in range(0,size,32)]
                    files[name]=control_movie(refine,camera=(scale,0,0),size=size,direct=direct,colored=True,vsp=True,illumination=flags,ic_mv=IC_MV,ic_mvp=1,merge_indices=indices)
                    values,motion,blocks,changed=oracle(size,scale);at=len(golden)//2;mt=len(vectors)//8
                    golden.extend(struct.pack('<'+str(len(values))+'H',*values));vectors.extend(struct.pack('<'+str(len(motion)*2)+'q',*(v for pair in motion for v in pair)))
                    cases.append(dict(file=name,size=size,direct=direct,refine=refine,scale=scale,oracle_offset=at,motion_offset=mt,blocks=blocks,changed_samples=changed))
    files['illumination-vsp.y16']=bytes(golden);files['illumination-vsp-motion.i64']=bytes(vectors)
    files['illumination-vsp.json']=(json.dumps(dict(provenance='Owned PCM/CABAC CU-level VSP/IC/VSP transitions with independent depth, motion, interpolation and IC boundary oracle; no private data or external codec.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
