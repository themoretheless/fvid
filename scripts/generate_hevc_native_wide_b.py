"""Owned native wide-DI/refined-IV B acceptance; no external media tools."""
import json,struct
import generate_hevc_refinement_b as r
from generate_hevc_refinement_b_colored import source,target
from generate_hevc_cqt import OUT

def generate():
    files={};cases=[];golden=bytearray()
    for weights in ['none','luma','color']:
        for mode in ['intra','bi','l0','l1','amp8','amp24']:
            for scale in [2147483647,-2147483647]:
                name=f'native-wide-b-{weights}-{mode}-{scale}.mp4'
                files[name]=r.movie(True,weights!='none',mode,scale,colored=True,explicit=weights=='color')
                disparity=(scale*254+64)>>7;assert abs(disparity)>2147483648
                offsets=[]
                for owner in ['source','target']:
                    offsets.append(len(golden)//2)
                    for component in range(3):
                        values=source(mode,component)if owner=='source'else target(mode,disparity,weights,component)[0]
                        golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                cases.append(dict(file=name,enabled=True,weights=weights,mode=mode,scale=scale,disparity=disparity,offsets=offsets))
    files['native-wide-b.y16']=bytes(golden)
    files['native-wide-b.json']=(json.dumps(dict(
        provenance='Owned mixed-role B pictures, valid signed Golomb camera extremes and scalar color/weight oracle; no external codec or test-time generation.',
        precision=0,cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
