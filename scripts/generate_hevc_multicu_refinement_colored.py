"""Owned multi-CU color refinement with independent four-tap chroma oracle."""
import json,struct
from generate_hevc_multicu_refinement import oracle
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
FILTERS=[(0,64,0,0),(-2,58,10,-2),(-4,54,16,-2),(-6,46,28,-4),(-4,36,36,-4),(-4,28,46,-6),(-2,16,54,-4),(-2,10,58,-2)]

def generate():
    files={};cases=[];golden=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for scale,offset in [(128,0),(-128,0),(4,0),(-4,0),(16,-3),(-16,3),(1,0),(-1,0)]:
                name=f'multicu-refinement-colored-{size}-{int(direct)}-{scale}-{offset}.mp4'
                y,blocks,_=oracle(size,scale,offset);at=len(golden)//2
                planes=[y];side=size//2;cols=size//32
                for component in [0,1]:
                    values=[]
                    for yy in range(side):
                        for xx in range(side):
                            mv=blocks[(yy*2//32)*cols+xx*2//32]['vector'];dx,phase=divmod(mv,8)
                            value=sum(w*((13*min(side-1,max(0,xx+dx+i-1))+7*yy+29+83*component)%256)for i,w in enumerate(FILTERS[phase]))
                            values.append(min(255,max(0,(value+32)>>6)))
                    planes.append(values)
                for plane in planes:golden.extend(struct.pack('<'+str(len(plane))+'H',*plane))
                files[name]=control_movie(True,camera=(scale,offset,0),size=size,direct=direct,colored=True,
                    merge_indices=[b['merge_index']for b in blocks])
                cases.append(dict(file=name,size=size,direct=direct,scale=scale,offset=offset,blocks=blocks,oracle_offset=at))
    files['multicu-refinement-colored.y16']=bytes(golden)
    files['multicu-refinement-colored.json']=(json.dumps(dict(
        provenance='Owned multi-CTU PCM/CABAC/MP4 with distinct source/target chroma. Independent corner-depth/disparity, luma and chroma filters; no external codecs or test-time generation.',cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
