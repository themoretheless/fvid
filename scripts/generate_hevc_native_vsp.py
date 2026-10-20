"""Owned coded VSP and an independent depth/sub-block/pixel oracle, offline."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_multicu_refinement import FILTERS as LUMA
from generate_hevc_multicu_refinement_colored import FILTERS as CHROMA
from generate_hevc_wedge_tables import OUT

def oracle(size,scale,offset,clip_output=True):
    motion=[0]*(size*size);blocks=[];wrong_refinement=0
    def depth(x,y):return(7*min(size-1,max(0,x))+11*min(size-1,max(0,y))+37)%256
    def camera(v):return(scale*v+offset*256+64)>>7
    for y in range(0,size,32):
        for x in range(0,size,32):
            base=motion[(y+31)*size+x-1]if x else motion[(y-1)*size+x+31]if y else 0
            dx=(base+2)>>2
            corners=[depth(x+dx,y),depth(x+dx+31,y),depth(x+dx,y+31),depth(x+dx+31,y+31)]
            horizontal=(corners[0]<corners[3])==(corners[1]<corners[2]);sw,sh=(8,4)if horizontal else(4,8)
            refined=camera(max(corners));bad_dx=(refined+2)>>2
            for sy in range(0,32,sh):
                for sx in range(0,32,sw):
                    d=max(depth(x+dx+sx+xx,y+sy+yy)for yy in [0,sh-1]for xx in [0,sw-1])
                    v=camera(d)
                    bad=camera(max(depth(x+bad_dx+sx+xx,y+sy+yy)for yy in [0,sh-1]for xx in [0,sw-1]))
                    wrong_refinement+=v!=bad
                    for yy in range(y+sy,y+sy+sh):
                        for xx in range(x+sx,x+sx+sw):motion[yy*size+xx]=v
            blocks.append(dict(origin=[x,y],base=base,horizontal=horizontal,subblock=[sw,sh]))
    pixels=[]
    for c in range(3):
        side=size if c==0 else size//2;divisor=4 if c==0 else 8;left=3 if c==0 else 1
        for y in range(side):
            for x in range(side):
                mv=motion[(y if c==0 else y*2)*size+(x if c==0 else x*2)]
                dx,phase=divmod(mv,divisor);weights=LUMA[phase]if c==0 else CHROMA[phase]
                def sample(xx):
                    xx=min(side-1,max(0,xx))
                    return(3*xx+5*y+17)%256 if c==0 else(13*xx+7*y+29+83*(c-1))%256
                value=sum(w*sample(x+dx+i-left)for i,w in enumerate(weights))
                pixels.append(min(255,max(0,(value+32)>>6)) if clip_output else (value+32)>>6)
    return pixels,motion,blocks,wrong_refinement

def generate():
    files={};cases=[];golden=bytearray();motions=bytearray();witnesses=0
    for size in [32,64,128]:
        for direct in [False,True]:
            for iv in [False,True]:
                for refine in [False,True]:
                    for scale,offset in [(4,0),(-4,0),(128,0),(-128,0),(0,0),(0,2)]:
                        name=f'native-vsp-{size}-{int(direct)}-{int(iv)}-{int(refine)}-{scale}-{offset}.mp4'
                        pixels,motion,blocks,count=oracle(size,scale,offset);witnesses+=count
                        at=len(golden)//2;mt=len(motions)//8
                        golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels));motions.extend(struct.pack('<'+str(len(motion))+'q',*motion))
                        indices=[int(b['origin'][0]==0 and b['origin'][1]>0)for b in blocks]
                        files[name]=control_movie(refine,camera=(scale,offset,0),size=size,direct=direct,colored=True,vsp=True,inter_view=iv,merge_indices=indices)
                        cases.append(dict(file=name,size=size,direct=direct,iv=iv,refine=refine,scale=scale,offset=offset,blocks=blocks,oracle_offset=at,motion_offset=mt))
    files['native-vsp-no-camera.mp4']=control_movie(False,camera_available=False,colored=True,vsp=True,inter_view=False)
    files['native-vsp.y16']=bytes(golden);files['native-vsp-motion.i64']=bytes(motions)
    files['native-vsp.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC/MP4 and independent base-DV, depth-corner split, VSP sub-block and luma/chroma filter oracle. No external codec, private data or test-time generation.',
        wrong_refinement_witnesses=witnesses,cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
