"""Owned P/B IC merge-zero-disable syntax and exact sample controls, offline."""
import json,struct
from generate_hevc_cqt import OUT
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import oracle as p_oracle
from generate_hevc_illumination_b import movie as b_movie,oracle as b_oracle,MODES
from generate_hevc_illumination_b_fractional import oracle as fractional_b_oracle

def generate(b_slice=False):
    files={};cases=[];golden=bytearray();suppression=0;gain=(2,1,-40)
    for size in [64,128]:
        for direct in [False,True]:
            for enabled in [False,True]:
                for disabled in [False,True]:
                    for fractional in [False,True]:
                        modes=MODES if b_slice else ['p']
                        for mode in modes:
                            name=f'illumination-merge-zero-{"b"if b_slice else"p"}-{size}-{int(direct)}-{int(enabled)}-{int(disabled)}-{int(fractional)}-{mode}.mp4'
                            if b_slice:
                                mvs=((1,3),(-3,-1))if fractional else None
                                files[name]=b_movie(size,direct,enabled,mode,gain,mvs,disabled)
                                if fractional:
                                    values,changed,_=fractional_b_oracle(size,enabled,mode,gain,mvs,disabled)
                                    ungated,_,_=fractional_b_oracle(size,enabled,mode,gain,mvs,False)
                                else:
                                    values,changed=b_oracle(size,enabled,mode,gain,disabled)
                                    ungated,_=b_oracle(size,enabled,mode,gain,False)
                            else:
                                mv=(3,-1)if fractional else None
                                indices=[2 if x==0 and y==0 else 1 if(x,y)in[(32,0),(0,32)]else 0 for y in range(0,size,32)for x in range(0,size,32)]
                                files[name]=control_movie(False,size=size,direct=direct,colored=True,illumination=enabled,target_luma=gain,ic_mv=mv,merge_indices=indices,ic_disabled_merge_zero=disabled)
                                args=dict(mv_horizontal=3 if fractional else 0,mv_vertical=-1 if fractional else 0)
                                values,changed=p_oracle(size,enabled,gain,merge_zero_disabled=disabled,**args)
                                ungated,_=p_oracle(size,enabled,gain,**args)
                            witnesses=sum(a!=b for a,b in zip(values,ungated));suppression+=witnesses
                            at=len(golden)//2;golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                            case=dict(file=name,size=size,direct=direct,enabled=enabled,target_luma=gain,disabled_merge_zero=disabled,fractional=fractional,mode=mode,oracle_offset=at,changed_samples=changed,suppression_witnesses=witnesses)
                            if b_slice:
                                case['references']=MODES[mode]
                                if fractional:case['motion']=mvs
                            elif fractional:case.update(mv_horizontal=3,mv_vertical=-1)
                            cases.append(case)
    prefix='illumination-merge-zero-'+('b'if b_slice else'p')
    files[prefix+'.y16']=bytes(golden)
    files[prefix+'.json']=(json.dumps(dict(provenance='Owned PCM/CABAC P/B slice IC merge-zero-disable presence and independent gated boundary/sample oracle. No external codec or private data.',suppression_witnesses=suppression,cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for b_slice in [False,True]:
        for name,data in generate(b_slice).items():(OUT/name).write_bytes(data)
