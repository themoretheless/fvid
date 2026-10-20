"""Owned short regressions separating Annex I merge scaling from AMVP."""
import json
from generate_hevc_wedge_tables import OUT
from generate_hevc_inter_view_scaling import generate_case
from generate_hevc_depth_view_scaling import generate_geometry

def generate():
    files={};cases=[];depth=generate_geometry()
    for kind in ['texture','depth']:
        for merge in [False,True]:
            texture=generate_case('',(100,102,108),4,merge=merge)if kind=='texture'else None
            for enabled in [False,True]:
                stem=('inter-view-temporal-scaling-'if kind=='texture'else'depth-view-temporal-scaling-')+('on'if enabled else'off')+('-merge'if merge else'')+'.mp4'
                name=f"amvp-merge-{kind}-{'on'if enabled else'off'}-{'merge'if merge else'amvp'}.mp4"
                files[name]=(texture if texture is not None else depth)[stem]
                cases.append(dict(file=name,kind=kind,merge=merge,enabled=enabled,target_layer=5 if kind=='texture'else 8,source_layer=3 if kind=='texture'else 5,
                    expected_quarter_mv=16 if merge and enabled else 4))
    files['amvp-merge-manifest.json']=(json.dumps(dict(provenance='Owned PCM/CABAC and MP4 only. I.8.5.3.2.5 explicitly selects base 8.5.3.2.9 for AMVP, which copies long-term motion; merge alone selects I.8.5.3.2.6 view scaling. No private media, FFmpeg, network or test-time generation.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
