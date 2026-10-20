"""Owned multi-CTU DI/depth-refinement streams, no external codec tools."""
import json
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT

def generate():
    files={};cases=[]
    for size in [64,128]:
        for direct in [False,True]:
            for offset in [0,2,-2,2147483647,-2147483647]:
                name=f'multicu-wide-{size}-{int(direct)}-{offset}.mp4'
                files[name]=control_movie(True,camera=(0,offset,0),size=size,direct=direct)
                cases.append(dict(file=name,size=size,direct=direct,offset=offset,disparity=offset*2))
    files['multicu-wide.json']=(json.dumps(dict(
        provenance='Owned mixed-role PCM/CABAC, multi-CTU skip target and MP4. Camera scale zero isolates constant signed disparity across CU boundaries. No private media, FFmpeg or test-time generation.',
        cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
