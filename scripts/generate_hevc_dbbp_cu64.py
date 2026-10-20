"""Owned 16x16 and 64x64 DBBP CU fixtures, offline PCM/CABAC/MP4."""
import json
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT

def generate(cu=64):
    cases=[]
    for size,direct in [(32 if cu==16 else 64,False),(128,False),(128,True)]:
        for vertical in [False,True]:
            for enabled in [False,True]:
                name=f'dbbp-cu{cu}-{size}-{int(direct)}-{int(vertical)}-{int(enabled)}.mp4'
                (OUT/name).write_bytes(control_movie(False,size=size,direct=direct,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_log=6 if cu==64 else 5,dbbp_cu_log=cu.bit_length()-1))
                cases.append(dict(file=name,size=size,direct=direct,vertical=vertical,enabled=enabled))
    (OUT/f'dbbp-cu{cu}.json').write_text(json.dumps(dict(provenance=f'Owned {cu}x{cu} CU DBBP and 32x32 PCM source leaves, offline; no external codec or private media.',cases=cases),indent=2)+'\n')
if __name__=='__main__':
    generate()
    generate(16)
