"""Owned DBBP B IV/default/L1 reduction, scalar fractional oracle, offline."""
import json,struct
from functools import lru_cache
from generate_hevc_dbbp_b import movie,iv_first_mvd
from generate_hevc_illumination_b import pattern
from generate_hevc_texture_iv_b import WEIGHTS
from generate_hevc_multicu_refinement import FILTERS as LUMA
from generate_hevc_multicu_refinement_colored import FILTERS as CHROMA
from generate_hevc_dbbp_residual import compose
from generate_hevc_wedge_tables import OUT
GAIN=(2,1,-40)

def clip(value):return min(255,max(0,value))
@lru_cache(None)
def internal(c,mv,bias,size=32):
    side=size if c==0 else size//2;den=4 if c==0 else 8;left=3 if c==0 else 1
    filters=LUMA if c==0 else CHROMA
    ix,fx=divmod(mv[0],den);iy,fy=divmod(mv[1],den)
    def at(x,y):return pattern(min(side-1,max(0,x)),min(side-1,max(0,y)),c,True,bias,GAIN)
    horizontal=[[sum(w*at(x+ix+k-left,y)for k,w in enumerate(filters[fx]))for x in range(side)]for y in range(side)]
    return [sum(w*horizontal[min(side-1,max(0,y+iy+k-left))][x]for k,w in enumerate(filters[fy]))>>6 for y in range(side)for x in range(side)]

def uni(values,c,weighted,list_index=0):
    w,o=WEIGHTS[list_index][0]if c==0 else(2,0)
    return [clip(((v*w+64)>>7)+o)if weighted else clip((v+32)>>6)for v in values]
def bi(first,second,c,weighted):
    w0,o0=WEIGHTS[0][0]if c==0 else(2,0)
    w1,o1=WEIGHTS[1][0]if c==0 else(2,0)
    return [clip((a*w0+b*w1+((o0+o1+1)<<7))>>8)if weighted else clip((a+b+64)>>7)for a,b in zip(first,second)]

def generate(l1_only=False):
    prefix='dbbp-b-iv-l1'if l1_only else'dbbp-b-iv'
    cases=[];golden=bytearray();subblock_witnesses=0;l1_witnesses=0
    configurations=[(v,e,w,c,a,False,32,False)for v in [False,True]for e in [False,True]for w in [False,True]for c in [8,24]for a in [(4,-8),(1,-3),(-7,5)]]
    configurations += [(False,e,w,24,a,True,32,False)for e in [False,True]for w in [False,True]for a in [(4,-8),(1,-3),(-7,5)]]
    configurations += [(v,e,w,24,(1,-3),False,128,d)for v in [False,True]for e in [False,True]for w in [False,True]for d in [False,True]]
    for vertical,enabled,weighted,cut,amplitudes,shifted,size,direct in configurations:
        name=prefix+'-'+(f'{size}-{int(direct)}-'if size!=32 else'')+f'{int(vertical)}-{int(enabled)}-{int(weighted)}-{cut}-{amplitudes[0]}-{amplitudes[1]}'+('-shift'if shifted else'')+'.mp4'
        (OUT/name).write_bytes(movie(vertical,enabled,GAIN,weighted,iv_cut=cut,amplitudes=amplitudes,iv_shift=shifted,size=size,direct=direct,l1_only=l1_only))
        first_list=int(l1_only and size>32)
        default_x=24 if vertical else 16;sign=-1 if shifted else 1 if default_x<cut else -1
        grid=[]
        for y in range(0,size,8):
            for x in range(0,size,8):
                second=x%32>=16 if vertical else y%32>=16
                selected_sign=sign if enabled or shifted else 1 if x%32<cut else -1
                grid.append([None if l1_only else[0,selected_sign*amplitudes[0]],[0,selected_sign*amplitudes[1]]]if second else([None,[0,0]]if first_list else[[0,0],None]))
        pixels=[]
        for c in range(3):
            side=size if c==0 else size//2;block=32 if c==0 else 16;scale=32//block
            first=uni(internal(c,(0,0),97 if first_list else 0,size),c,weighted,first_list)
            plus=bi(internal(c,(0,amplitudes[0]),0,size),internal(c,(0,amplitudes[1]),97,size),c,weighted)
            minus=bi(internal(c,(0,-amplitudes[0]),0,size),internal(c,(0,-amplitudes[1]),97,size),c,weighted)
            if l1_only:
                plus=uni(internal(c,(0,amplitudes[1]),97,size),c,weighted,1)
                minus=uni(internal(c,(0,-amplitudes[1]),97,size),c,weighted,1)
            subblocks=[(plus if (x*scale)%32<cut else minus)[y*side+x]for y in range(side)for x in range(side)]
            default=uni(internal(c,(0,sign*amplitudes[0]),0,size),c,weighted)
            if l1_only:default=uni(internal(c,(0,sign*amplitudes[1]),97,size),c,weighted,1)
            selected=default if enabled else (plus if sign>0 else minus)if shifted else subblocks
            output=[0]*(side*side)
            for by in range(0,side,block):
                for bx in range(0,side,block):
                    get=lambda plane:[plane[(by+y)*side+bx+x]for y in range(block)for x in range(block)]
                    part=compose([get(first),get(selected)],c,vertical,enabled,(bx*scale,by*scale))
                    if enabled:
                        wrong=compose([get(first),[0]*(block*block)if l1_only else get(plus if sign>0 else minus)],c,vertical,True,(bx*scale,by*scale))
                        l1_witnesses+=sum(a!=b for a,b in zip(part,wrong))
                        subblock_witnesses+=sum(a!=b for a,b in zip(part,compose([get(first),get(subblocks)],c,vertical,True,(bx*scale,by*scale))))
                    for y in range(block):output[(by+y)*side+bx:(by+y)*side+bx+block]=part[y*block:(y+1)*block]
            pixels.extend(output)
        offset=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
        cases.append(dict(file=name,vertical=vertical,enabled=enabled,weighted=weighted,cut=cut,amplitudes=amplitudes,shifted=shifted,size=size,direct=direct,motion_grid=grid,first_list=first_list,first_mvd=iv_first_mvd(size,vertical,enabled,cut,(amplitudes[1],amplitudes[0])if first_list else amplitudes,shifted),offset=offset))
    assert subblock_witnesses>0 and l1_witnesses>0
    (OUT/f'{prefix}.y16').write_bytes(golden)
    (OUT/f'{prefix}.json').write_text(json.dumps(dict(provenance='Owned B AMP source and DBBP IV target, PCM references and scalar separable convolution, weighting and contour oracle. Offline.',subblock_witnesses=subblock_witnesses,**{'l1_retention_witnesses'if l1_only else'l1_reduction_witnesses':l1_witnesses},cases=cases),indent=2)+'\n')
if __name__=='__main__':
    generate()
    generate(l1_only=True)
