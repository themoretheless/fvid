"""Offline owned coded residual B lists and independent integer sample oracle."""
import json,struct
from generate_hevc_illumination_b import movie,pattern,MODES as BASE_MODES
from generate_hevc_texture_iv_b import WEIGHTS
from generate_hevc_texture_iv_chroma_b import CHROMA
from generate_hevc_cqt import OUT
MODES={**BASE_MODES,'future-past':(2,1)}

def combine(values,refs,c,weighted,replace=False):
    active=[(i,index)for i,index in enumerate(refs)if index is not None]
    if not weighted:return min(255,max(0,(sum(values)+(32 if len(values)==1 else 64))>>(6 if len(values)==1 else 7)))
    factors=[WEIGHTS[i][0 if replace else index]if c==0 else CHROMA[i][0 if replace else index][c-1]for i,index in active]
    if len(values)==1:
        weight,offset=factors[0];result=((values[0]*weight+64)>>7)+offset
    else:
        (a,oa),(b,ob)=factors;result=(values[0]*a+values[1]*b+((oa+ob+1)<<7))>>8
    return min(255,max(0,result))

def oracle(size,mode,gain,weights,weighted):
    result=[];late_clipping=0;original_indices=0
    for c in range(3):
        side=size if c==0 else size//2;cu=32 if c==0 else 16
        for y in range(side):
            for x in range(side):
                first=x<cu and y<cu;refs=(0,0)if first else MODES[mode]
                weight=weights[(y//cu)*(size//32)+x//cu];values=[]
                cur=pattern(x,y,c,False,0,gain)
                for direction,index in enumerate(refs):
                    if index is None:continue
                    iv=index==(1 if direction==0 else 2)
                    if not weight:
                        bias=97 if(direction==0 and index==2)or(direction==1 and index==0)else 0
                        raw=(cur if iv else pattern(x,y,c,True,bias,gain))*64
                    elif not iv:
                        bias=97 if direction else 0
                        base=pattern(x,y,c,True,bias,gain)
                        ref=pattern(x,y,c,False,bias,gain)
                        raw=base*64+(((cur-ref)*64)>>(weight-1))
                    else:
                        other=refs[1-direction]
                        other_iv=other==(1 if 1-direction==0 else 2)
                        z=1-direction if other is not None and not other_iv else 0 if other is not None else direction
                        bias=97 if z else 0
                        a=pattern(x,y,c,True,bias,gain);b=pattern(x,y,c,False,bias,gain)
                        raw=cur*64+(((a-b)*64)>>(weight-1))
                    values.append(raw)
                expected=combine(values,refs,c,weighted)
                late_clipping+=weight!=0 and expected!=combine([min(255*64,max(0,v))for v in values],refs,c,weighted)
                original_indices+=weighted and weight!=0 and expected!=combine(values,refs,c,weighted,True)
                result.append(expected)
    return result,late_clipping,original_indices

def generate():
    cases=[];golden=bytearray();witnesses=0;indices=0;gain=(2,1,-40)
    for size in [64,128]:
        for direct in [False,True]:
            for mode,refs in MODES.items():
                for weight in [0,1,2,'mixed']:
                    weights=[0,1,2,1]*((size//32)**2//4)if weight=='mixed'else[weight]*((size//32)**2)
                    for weighted in [False,True]:
                        name=f'residual-b-{size}-{int(direct)}-{mode}-{weight}{"-weighted"if weighted else""}.mp4'
                        (OUT/name).write_bytes(movie(size,direct,False,mode,gain,residual_prediction=weights,weighted=weighted,references=refs))
                        pixels,changed,indexed=oracle(size,mode,gain,weights,weighted);at=len(golden)//2;witnesses+=changed;indices+=indexed
                        golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
                        cases.append(dict(file=name,size=size,direct=direct,mode=mode,weighted=weighted,weights=weights,references=refs,oracle_offset=at,original_weight_index_witnesses=indexed,late_combination_witnesses=changed))
    (OUT/'residual-b.y16').write_bytes(golden)
    (OUT/'residual-b.json').write_text(json.dumps(dict(provenance='Owned PCM/CABAC/MP4, independent integer per-list residual/explicit-weight oracle. No external codecs, private data or runtime generation.',late_combination_witnesses=witnesses,original_weight_index_witnesses=indices,cases=cases),indent=2)+'\n')
if __name__=='__main__':generate()
