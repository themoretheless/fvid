"""Owned DBBP with signed transform residual, independent sample oracle."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_dbbp_fractional import predictor
from generate_hevc_wedge_tables import OUT
from generate_hevc_native_vsp_ac import BASIS

def compose(predictions,c,vertical,enabled,origin=(0,0)):
    side=32 if c==0 else 16;scale=32//side
    ox,oy=origin
    threshold=sum((7*(ox+x)+11*(oy+y)+37)%256 for x,y in [(0,0),(31,0),(0,31),(31,31)])//4
    label=lambda x,y:(7*(ox+x)+11*(oy+y)+37)%256>threshold
    combined=[];output=[]
    for y in range(side):
        for x in range(side):
            second=label(x*scale,y*scale)!=label(0,0)if enabled else(x if vertical else y)>=side//2
            combined.append(predictions[int(second)][y*side+x])
    for y in range(side):
        for x in range(side):
            px,py,nx,ny=(max(0,x-1),y,min(side-1,x+1),y)if vertical else(x,max(0,y-1),x,min(side-1,y+1))
            nxt=label(min(31,x*scale+scale),y*scale)if vertical else label(x*scale,min(31,y*scale+scale))
            value=combined[y*side+x]
            if enabled and label(px*scale,py*scale)!=nxt:value=(combined[py*side+px]+2*value+combined[ny*side+nx])//4
            output.append(value)
    return output

def generate(ac=None):
    prefix=f'dbbp-ac-{ac}'if ac else'dbbp-residual'
    cases=[];golden=bytearray();order_witnesses=0;nonzero=0;motion_witnesses=0
    mvs=[(1,3),(-7,-5)]
    for vertical in [False,True]:
        for enabled in [False,True]:
            for weighted in [False,True]:
                weights=[[(3,-50),(1,20),(-1,100)],[(-2,90),(4,-60),(3,-80)]]if weighted else None
                for levels in [(64,64,-64),(-64,-64,64),(0,64,64),(0,-64,-64)]:
                    name=f'{prefix}-{int(vertical)}-{int(enabled)}-{int(weighted)}-'+'-'.join(map(str,levels))+'.mp4'
                    (OUT/name).write_bytes(control_movie(False,size=32,colored=True,inter_view=False,target_luma=(2,1,-40),dbbp=(vertical,enabled),dbbp_mvs=mvs,dbbp_weights=weights,dbbp_residual=levels,dbbp_ac=ac))
                    pixels=[]
                    for c in range(3):
                        predictions=[predictor(c,mvs[i],i==0,weights[i][c]if weighted else None)for i in range(2)]
                        side=32 if c==0 else 16
                        if ac:
                            dequant=(3264 if c==0 else 6528)*(1 if levels[c]>0 else -1)if levels[c] else 0
                            if ac=='vertical':
                                adjustments=[(64*((BASIS[side][y]*dequant+64)>>7)+2048)>>12 for y in range(side)for x in range(side)]
                            else:
                                first=(64*dequant+64)>>7
                                adjustments=[(BASIS[side][x]*first+2048)>>12 for y in range(side)for x in range(side)]
                        else:
                            adjustment=(26 if levels[c]>0 else -25)if c==0 and levels[c] else (51 if levels[c]>0 else -51)if levels[c] else 0
                            adjustments=[adjustment]*(side*side)
                        base=compose(predictions,c,vertical,enabled)
                        output=[min(255,max(0,value+residual))for value,residual in zip(base,adjustments)]
                        wrong=compose([[min(255,max(0,value+residual))for value,residual in zip(plane,adjustments)]for plane in predictions],c,vertical,enabled)
                        order_witnesses+=sum(a!=b for a,b in zip(output,wrong))
                        nonzero+=sum(a!=b for a,b in zip(output,base))
                        zero_motion=compose([predictor(c,(0,0),i==0,weights[i][c]if weighted else None)for i in range(2)],c,vertical,enabled)
                        zero_output=[min(255,max(0,value+residual))for value,residual in zip(zero_motion,adjustments)]
                        motion_witnesses+=sum(a!=b for a,b in zip(output,zero_output))
                        pixels.extend(output)
                    offset=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                    cases.append(dict(file=name,vertical=vertical,enabled=enabled,weighted=weighted,levels=levels,ac=ac,mvs=mvs,references=[0,1],offset=offset))
    assert order_witnesses>0 and nonzero>0
    (OUT/f'{prefix}.y16').write_bytes(golden)
    (OUT/f'{prefix}.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4 with signed DC or first-frequency AC coefficients and scalar interpolation/composition/inverse-DCT oracle. Offline.',different_from_zero_motion=motion_witnesses,nonzero_residual_witnesses=nonzero,residual_order_witnesses=order_witnesses,cases=cases),indent=2)+'\n')
if __name__=='__main__':
    generate()
    generate('vertical')
    generate('horizontal')
