"""Own explicit horizontal fractional inter-view MV with IC, offline."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import oracle
from generate_hevc_cqt import OUT

def generate(two_d=False):
    files={};cases=[];golden=bytearray()
    for size in [64,128]:
        for direct in [False,True]:
            for enabled in [False,True]:
                for mv,vertical in ([(0,y)for y in [-7,-3,-1,1,2,3,4,5,6,7]]+[(p,(3*p)%8)for p in range(1,8)]+[(-1,3),(3,-1),(-7,-5)] if two_d else [(x,0)for x in [-7,-5,-3,-1,1,2,3,4,5,6,7]]):
                    gain=(2,1,-40)
                    prefix='illumination-2d' if two_d else 'illumination-fractional'
                    name=f'{prefix}-{size}-{int(direct)}-{int(enabled)}-{mv}'+(f'-{vertical}'if two_d else'')+'.mp4'
                    indices=[2 if x==0 and y==0 else 1 if(x,y)in[(32,0),(0,32)]else 0 for y in range(0,size,32)for x in range(0,size,32)]
                    files[name]=control_movie(False,colored=True,size=size,direct=direct,illumination=enabled,target_luma=gain,merge_indices=indices,ic_mv=(mv,vertical))
                    values,changed=oracle(size,enabled,gain,mv_horizontal=mv,mv_vertical=vertical);at=len(golden)//2
                    golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                    case=dict(file=name,size=size,direct=direct,enabled=enabled,target_luma=gain,mv_horizontal=mv,oracle_offset=at,changed_samples=changed)
                    if two_d:case["mv_vertical"]=vertical
                    cases.append(case)
    files[prefix+'.y16']=bytes(golden)
    files[prefix+'.json']=(json.dumps(dict(provenance='Owned explicit inter-view P MV syntax, PCM/CABAC and independent horizontal luma/chroma interpolation and IC boundary oracle. No external codec or private data.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate(True).items():(OUT/name).write_bytes(data)
