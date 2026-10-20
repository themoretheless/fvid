"""Offline owned DBBP fractional predictors and scalar convolution oracle."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_wedge_tables import OUT
from generate_hevc_multicu_refinement import FILTERS as LUMA
from generate_hevc_multicu_refinement_colored import FILTERS as CHROMA

def predictor(component,mv,own,weights=None,size=32):
    side=size if component==0 else size//2
    den=4 if component==0 else 8
    left=3 if component==0 else 1
    filters=LUMA if component==0 else CHROMA
    ix,fx=divmod(mv[0],den);iy,fy=divmod(mv[1],den)
    def sample(x,y):
        x=min(side-1,max(0,x));y=min(side-1,max(0,y))
        v=(3*x+5*y+17)%256 if component==0 else (13*x+7*y+29+83*(component-1))%256
        return min(255,max(0,2*v-40)) if own and component==0 else (v+([71,113][component-1]if own else 0))%256 if component else v
    horizontal=[[sum(w*sample(x+ix+k-left,y)for k,w in enumerate(filters[fx]))for x in range(side)]for y in range(side)]
    # At eight bits the separable internal intermediate has shift zero;
    # final uni prediction rounds the 12-bit filter scale only once.
    values=[sum(w*horizontal[min(side-1,max(0,y+iy+k-left))][x]for k,w in enumerate(filters[fy]))for y in range(side)for x in range(side)]
    if weights is None:return [min(255,max(0,(raw+2048)>>12))for raw in values]
    weight,offset=weights
    return [min(255,max(0,(((raw>>6)*weight+64)>>7)+offset))for raw in values]

def generate(merge=False,weighted=False):
    cases=[];golden=bytearray();witnesses=0;rounding_witnesses=0
    for vertical in [False,True]:
        for enabled in [False,True]:
            configurations=[(((1,3),(0,0)),index)for index in [0,1]]if merge else [(mvs,None)for mvs in [((1,3),(-7,-5)),((-3,2),(7,5)),((0,-7),(-1,0)),((7,7),(3,-1))]]
            for mvs,merge_index in configurations:
                prefix='dbbp-weighted'if weighted else'dbbp-merge'if merge else'dbbp-fractional'
                name=f'{prefix}-{int(vertical)}-{int(enabled)}-'+ '-'.join(str(v)for mv in mvs for v in mv)+(f'-merge{merge_index}'if merge else'')+'.mp4'
                weights=[[(3,-50),(1,20),(-1,100)],[(-2,90),(4,-60),(3,-80)]]if weighted else None
                (OUT/name).write_bytes(control_movie(False,size=32,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_mvs=mvs,dbbp_merge=merge_index,dbbp_weights=weights))
                pixels=[]
                for c in range(3):
                    side=32 if c==0 else 16;scale=32//side
                    predictions=[predictor(c,mvs[0],True,weights[0][c]if weighted else None),predictor(c,mvs[1],merge and merge_index==0,weights[1][c]if weighted else None)]
                    unweighted=[predictor(c,mvs[0],True),predictor(c,mvs[1],merge and merge_index==0)]
                    zero=[predictor(c,(0,0),True,weights[0][c]if weighted else None),predictor(c,(0,0),merge and merge_index==0,weights[1][c]if weighted else None)]
                    threshold=sum((7*x+11*y+37)%256 for x,y in [(0,0),(31,0),(0,31),(31,31)])//4
                    label=lambda x,y:(7*x+11*y+37)%256>threshold
                    combined=[]
                    for y in range(side):
                        for x in range(side):
                            second=label(x*scale,y*scale)!=label(0,0)if enabled else (x if vertical else y)>=side//2
                            value=predictions[int(second)][y*side+x];combined.append(value)
                            witnesses+=value!=zero[int(second)][y*side+x]
                            if weighted:
                                weight,offset=weights[int(second)][c]
                                early=min(255,max(0,((unweighted[int(second)][y*side+x]*weight+1)>>1)+offset))
                                rounding_witnesses+=value!=early
                    for y in range(side):
                        for x in range(side):
                            px,py,nx,ny=(max(0,x-1),y,min(side-1,x+1),y)if vertical else(x,max(0,y-1),x,min(side-1,y+1))
                            nxt=label(min(31,x*scale+scale),y*scale)if vertical else label(x*scale,min(31,y*scale+scale))
                            value=combined[y*side+x]
                            if enabled and label(px*scale,py*scale)!=nxt:value=(combined[py*side+px]+2*value+combined[ny*side+nx])//4
                            pixels.append(value)
                offset=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                cases.append(dict(file=name,vertical=vertical,enabled=enabled,mvs=mvs,references=[0,merge_index if merge else 1],offset=offset))
    assert witnesses>0
    (OUT/f'{prefix}.y16').write_bytes(golden)
    (OUT/f'{prefix}.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4 and scalar separable convolution, no external codec or private media.',different_from_zero_motion=witnesses,early_rounding_witnesses=rounding_witnesses,cases=cases),indent=2)+'\n')
if __name__=='__main__':
    generate()
    generate(merge=True)
    generate(weighted=True)
