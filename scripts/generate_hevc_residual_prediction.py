"""Offline owned PCM/CABAC residual-prediction movies; no external codecs."""
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
import json

def generate():
    OUT.mkdir(parents=True,exist_ok=True)
    cases=[]
    for size in [64,128]:
        for direct in [False,True]:
            for weight in [0,1,2,'mixed']:
                weights=[0,1,2,1]*((size//32)**2//4)if weight=='mixed' else [weight]*((size//32)**2)
                for ic in [False,True]:
                    for bias in [20,-80,200]:
                        suffix=''if bias==20 else f'-bias{bias}'
                        name=f'residual-p-{size}-{int(direct)}-{weight}{"-ic"if ic else""}{suffix}.mp4'
                        (OUT/name).write_bytes(control_movie(False,colored=True,size=size,direct=direct,
                            inter_view=False,residual_prediction=weights,source_luma=(1,1,bias),target_luma=(1,2,30),ic_mv=(0,0),
                            illumination=[w!=0 for w in weights]if ic else None))
                        cases.append(dict(file=name,size=size,direct=direct,weights=weights,ic=ic,source_bias=bias))
    (OUT/'residual-p.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC, SPS and MP4; synthetic patterns only; offline generator.',cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
