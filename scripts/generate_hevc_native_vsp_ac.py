"""Own non-DC VSP residuals and normative single-frequency inverse DCT oracle."""
import json,struct
from generate_hevc_native_vsp_b import movie
from generate_hevc_native_vsp import oracle
from generate_hevc_cqt import OUT
# H.265 (08/2021) 8-319/8-321: first vertical AC basis, independent constants.
BASIS={32:[90,90,88,85,82,78,73,67,61,54,46,38,31,22,13,4,-4,-13,-22,-31,-38,-46,-54,-61,-67,-73,-78,-82,-85,-88,-90,-90],
       16:[90,87,80,70,57,43,25,9,-9,-25,-43,-57,-70,-80,-87,-90]}

def generate():
    files={};cases=[];golden=bytearray()
    for l1 in [False,True]:
        for scale in [4,-4,128,-128,0]:
            for levels in [(64,64,-64),(-64,-64,64),(0,64,64),(0,-64,-64)]:
                name=f'native-vsp-ac-{int(l1)}-{scale}-'+ '-'.join(map(str,levels))+'.mp4'
                files[name]=movie(scale,False,l1,levels,colored=True,ac=True)
                pixels,motion,_,_=oracle(32,scale,0);values=[];position=0
                for c,level in enumerate(levels):
                    side=32 if c==0 else 16
                    dequant=0 if level==0 else (3264 if c==0 else 6528)*(1 if level>0 else -1)
                    for y in range(side):
                        first=(BASIS[side][y]*dequant+64)>>7
                        residual=(64*first+2048)>>12
                        for x in range(side):
                            values.append(min(255,max(0,pixels[position+y*side+x]+residual)))
                    position+=side*side
                at=len(golden)//2;golden.extend(struct.pack('<1536H',*values))
                cases.append(dict(file=name,l1=l1,scale=scale,residual=levels,oracle_offset=at,motion=motion))
    files['native-vsp-ac.y16']=bytes(golden)
    files['native-vsp-ac.json']=(json.dumps(dict(provenance='Owned non-skip VSP B slices, single vertical AC coefficient at (0,1), independent two-pass normative DCT and VSP oracle. No external codec or private data.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
