"""Own coded IC non-unity gain, offset and saturation matrix, offline."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import oracle
from generate_hevc_cqt import OUT

def generate():
    files={};cases=[];golden=bytearray();all_gains=[]
    for size in [64,128]:
        for direct in [False,True]:
            for enabled in [False,True]:
                for gain in [(2,1,-40),(1,2,30),(0,1,200)]:
                    name=f'illumination-gain-{size}-{int(direct)}-{int(enabled)}-'+ '-'.join(map(str,gain))+'.mp4'
                    indices=[2 if x==0 and y==0 else 1 if(x,y)in[(32,0),(0,32)]else 0 for y in range(0,size,32)for x in range(0,size,32)]
                    files[name]=control_movie(False,colored=True,size=size,direct=direct,illumination=enabled,target_luma=gain,merge_indices=indices)
                    gains=[];values,changed=oracle(size,enabled,gain,gains);all_gains.extend(gains)
                    at=len(golden)//2;golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                    cases.append(dict(file=name,size=size,direct=direct,enabled=enabled,target_luma=gain,oracle_offset=at,changed_samples=changed,gains=gains))
    files['illumination-gain.y16']=bytes(golden)
    files['illumination-gain.json']=(json.dumps(dict(provenance='Owned synthetic PCM/CABAC IC P movies and independent boundary regression/saturation oracle. No external codecs or private data.',nonunity_gains=sum(g!=32 for g in all_gains),cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
