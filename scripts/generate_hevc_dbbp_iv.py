"""Owned DBBP IV-default versus IV-subblock witnesses, offline."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_dbbp_fractional import predictor
from generate_hevc_wedge_tables import OUT

def generate():
    cases=[];golden=bytearray();different=0;zero_witnesses=0
    configurations=[(v,e,c,False)for v in [False,True]for e in [False,True]for c in [8,24]]
    configurations += [(False,e,24,True)for e in [False,True]]
    for vertical,enabled,cut,shifted in configurations:
        name=f'dbbp-iv-{int(vertical)}-{int(enabled)}-{cut}'+('-shift'if shifted else'')+'.mp4'
        (OUT/name).write_bytes(control_movie(False,size=32,colored=True,source_cut=cut,inter_view=True,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_mvs=[(1,3),(0,0)],dbbp_merge=2 if shifted else 0))
        # The default source subblock is the centre 8x8 block of PU1.
        default_x=24 if vertical else 16
        default_mv=[0,-4 if shifted else 4 if default_x<cut else -4]
        def iv_mv(x):return [0,4 if (x//8)*8<cut else -4]
        grid=[]
        for y in range(0,32,8):
            for x in range(0,32,8):
                second=x>=16 if vertical else y>=16
                grid.append((default_mv if enabled or shifted else iv_mv(x))if second else [1,3])
        pixels=[]
        for c in range(3):
            side=32 if c==0 else 16;scale=32//side
            first=predictor(c,(1,3),True)
            zero=predictor(c,(0,0),True)
            up=predictor(c,(0,4),True);down=predictor(c,(0,-4),True)
            default=up if default_mv[1]>0 else down
            threshold=sum((7*x+11*y+37)%256 for x,y in [(0,0),(31,0),(0,31),(31,31)])//4
            label=lambda x,y:(7*x+11*y+37)%256>threshold
            combined=[]
            for y in range(side):
                for x in range(side):
                    second=label(x*scale,y*scale)!=label(0,0)if enabled else(x if vertical else y)>=side//2
                    i=y*side+x
                    subblocks=up if iv_mv(x*scale)[1]>0 else down
                    value=(default if enabled or shifted else subblocks)[i]if second else first[i]
                    combined.append(value)
                    zero_witnesses+=value!=zero[i]
                    different+=enabled and second and value!=subblocks[i]
            for y in range(side):
                for x in range(side):
                    px,py,nx,ny=(max(0,x-1),y,min(side-1,x+1),y)if vertical else(x,max(0,y-1),x,min(side-1,y+1))
                    nxt=label(min(31,x*scale+scale),y*scale)if vertical else label(x*scale,min(31,y*scale+scale))
                    value=combined[y*side+x]
                    if enabled and label(px*scale,py*scale)!=nxt:value=(combined[py*side+px]+2*value+combined[ny*side+nx])//4
                    pixels.append(value)
        offset=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
        cases.append(dict(file=name,vertical=vertical,enabled=enabled,mvs=[[1,3],default_mv],references=[0,0],motion_grid=grid,offset=offset))
    assert different>0
    (OUT/'dbbp-iv.y16').write_bytes(golden)
    (OUT/'dbbp-iv.json').write_text(json.dumps(dict(provenance='Owned source AMP and target DBBP CABAC/PCM/MP4; independent scalar convolution. Offline.',different_from_zero_motion=zero_witnesses,different_from_subblock_motion=different,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
