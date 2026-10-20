"""Owned DBBP two-PU P movies, offline native playback acceptance fixtures."""
import json
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
from generate_hevc_3d_sps import payload
from generate_hevc_colour_plane_fixtures import packed

def generate():
    cases=[]
    for size,direct in [(32,False),(128,False),(128,True)]:
        for vertical in [False,True]:
            for enabled in [False,True]:
                suffix='-direct' if direct else ''
                name=f'dbbp-p-{size}-{int(vertical)}-{int(enabled)}{suffix}.mp4'
                (OUT/name).write_bytes(control_movie(False,size=size,direct=direct,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled)))
                cases.append(dict(file=name,size=size,direct=direct,vertical=vertical,enabled=enabled))
    flags=[False]*14;flags[5]=True;bits=payload(flags)
    (OUT/'dbbp-syntax.bin').write_bytes(packed(bits))
    (OUT/'dbbp.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC, two explicit PUs, DBBP SPS and MP4. Offline, no external codec or private source.',syntax_bits=len(bits),cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
