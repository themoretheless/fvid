"""Owned multiblock refined IV, exact AMP motion and saved skip disparity oracle."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT

def oracle(size,cut,scale,offset):
    cols=size//32;blocks=[]
    for cy in range(cols):
        for cx in range(cols):
            x=cx*32;y=cy*32
            # No temporal/direct DV; A1's saved skip IV is eligible. B1 skip IV
            # is across a CTU row and cannot supply this temporal fallback.
            base=blocks[-1]['refined']if cx else 0
            dx=(base+2)>>2
            maximum=max((7*min(size-1,max(0,x+dx+xx))+11*(y+yy)+37)%256 for yy in [0,31]for xx in [0,31])
            blocks.append(dict(origin=[x,y],base=base,depth=maximum,refined=(scale*maximum+offset*256+64)>>7))
    planes=[]
    for target in [False,True]:
        for component in range(3):
            side=size if component==0 else size//2;values=[]
            for y in range(side):
                for x in range(side):
                    lx=x if component==0 else x*2;ly=y if component==0 else y*2
                    if target:
                        state=blocks[(ly//32)*cols+lx//32]
                        source_x=min(size-1,max(0,(((lx//8*8+4+((state['refined']+2)>>2))>>3)<<3)))
                    else:source_x=lx
                    dy=1 if source_x%32<cut else -1
                    if component==0:value=(3*x+5*min(size-1,max(0,y+dy))+17)%256
                    else:
                        base_y=y+(-1 if dy<0 else 0)
                        bias=29+83*(component-1)+([71,113][component-1]if target else 0)
                        total=sum(w*((13*x+7*min(side-1,max(0,base_y+i-1))+bias)%256)for i,w in enumerate([-4,36,36,-4]))
                        value=min(255,max(0,(total+32)>>6))
                    values.append(value)
            planes.extend(values)
    return planes,blocks

def generate():
    files={};cases=[];golden=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for cut in [8,24]:
                for scale,offset in [(128,0),(-128,0),(4,0),(-4,0),(16,-3),(-16,3)]:
                    name=f'multicu-iv-{size}-{int(direct)}-{cut}-{scale}-{offset}.mp4'
                    values,blocks=oracle(size,cut,scale,offset);at=len(golden)//2
                    golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                    files[name]=control_movie(True,camera=(scale,offset,0),source_cut=cut,colored=True,size=size,direct=direct)
                    cases.append(dict(file=name,size=size,direct=direct,cut=cut,scale=scale,offset=offset,blocks=blocks,oracle_offset=at))
    files['multicu-iv.y16']=bytes(golden)
    files['multicu-iv.json']=(json.dumps(dict(provenance='Owned mixed-role PCM/CABAC and MP4. Independent saved A1 disparity, shifted corner maximum, 8x8 source-motion lookup and own-view chroma filtering. No external codec or test-time generation.',cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
