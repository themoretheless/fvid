"""Own non-skip VSP merge plus signed ordinary transform residual, offline."""
import json,struct
from generate_hevc_native_vsp_b import movie
from generate_hevc_native_vsp import oracle
from generate_hevc_cqt import OUT

def generate(chroma=False):
    files={};cases=[];golden=bytearray()
    for l1 in [False,True]:
        for scale in [4,-4,128,-128,0]:
            for residual in ([(64,64,-64),(-64,-64,64),(0,64,64),(0,-64,-64)] if chroma else [64,-64]):
                label='-'.join(map(str,residual)) if chroma else str(residual)
                name=f'native-vsp-residual{"-chroma" if chroma else ""}-{int(l1)}-{scale}-{label}.mp4'
                files[name]=movie(scale,False,l1,residual,colored=chroma)
                pixels,motion,_,_=oracle(32,scale,0)
                # QP26 dequantized DC +/-3264; two inverse DCT rounds give +26/-25.
                levels=residual if chroma else (residual,0,0)
                adjustment=26 if levels[0]>0 else -25 if levels[0]<0 else 0
                values=[min(255,max(0,v+adjustment))for v in pixels[:1024]]
                for c in [1,2]:
                    shift=51 if levels[c]>0 else -51 if levels[c]<0 else 0
                    predicted=pixels[1024+(c-1)*256:1024+c*256] if chroma else [128]*256
                    values.extend(min(255,max(0,v+shift)) for v in predicted)
                at=len(golden)//2;golden.extend(struct.pack('<1536H',*values))
                cases.append(dict(file=name,l1=l1,scale=scale,residual=residual,oracle_offset=at,motion=motion))
    prefix='native-vsp-residual-chroma' if chroma else 'native-vsp-residual'
    files[prefix+'.y16']=bytes(golden)
    files[prefix+'.json']=(json.dumps(dict(provenance='Owned CABAC full-merge non-skip VSP and ordinary signed DC transform residual; independent scalar VSP and inverse-transform oracle. No external codec or private data.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for chroma in [False,True]:
        for name,data in generate(chroma).items():(OUT/name).write_bytes(data)
