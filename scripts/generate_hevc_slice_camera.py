"""Owned I-only camera-header reproducer; isolates CP syntax from inter flags."""
import json
from generate_hevc_colour_plane_fixtures import packed
from generate_hevc_layered_pps import se
from generate_hevc_inter_view_scaling import generate_case
from generate_hevc_camera_parameters import camera_nal,replace_vps
from generate_hevc_wedge_tables import OUT
VALUES={3:[[1,0,0,0]],5:[[2,1,-1,3],[-3,-1,2,4]]}
LIMITS={3:[[2147483647,-2147483647,0,1]],5:[[2147483647,0,-2147483647,1],[0,2147483647,-1,0]]}
def generate():
    files={}
    for name,camera,views in [
        ('control',None,[(False,[]),(False,[])]),
        ('deferred',VALUES,[(True,[(0,None)]),(True,[(0,None),(1,None)])]),
        ('limits',LIMITS,[(True,[(0,None)]),(True,[(0,None),(1,None)])]),
    ]:
        movie=generate_case('',(100,102,108),4,all_intra=True,camera=camera)['inter-view-temporal-scaling-on.mp4']
        files['slice-camera-'+name+'.mp4']=replace_vps(movie,camera_nal(5 if name=='limits'else 0,views))
    files['slice-camera-values-layer3.bin']=packed(''.join(se(v)for v in VALUES[3][0]))
    files['slice-camera-values-prefixed.bin']=packed('101'+''.join(se(v)for v in VALUES[3][0]))
    files['slice-camera-manifest.json']=(json.dumps(dict(provenance='Owned independent I slices isolate header camera syntax, with no IC or merge fields. No external codec, private data, network or test-time fixture generation.',values=VALUES,limits=LIMITS),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
