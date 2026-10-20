"""Owned colored texture IV B streams and independent H.265 chroma oracle."""
import json,struct
from functools import lru_cache
import generate_hevc_texture_iv_b as b
from generate_hevc_cqt import banks,OUT
from generate_hevc_colour_plane_fixtures import packed,escape
from hevc_fixture_mp4 import mux

# H.265 v8 equations 8-241..8-261. With BitDepthC=8 the first filter
# has shift1=0 and the second has shift2=6; do not round between predictors.
FILTERS=[(0,64,0,0),(-2,58,10,-2),(-4,54,16,-2),(-6,46,28,-4),
         (-4,36,36,-4),(-4,28,46,-6),(-2,16,54,-4),(-2,10,58,-2)]
# Per list/reference, then Cb/Cr (weight, final decoded offset).
CHROMA=[
    [[(1,2),(3,-5)],[(2,-3),(0,11)],[(-1,7),(2,1)]],
    [[(3,-1),(-1,8)],[(1,4),(2,-4)],[(0,9),(1,6)]],
]

def weighted_header(data,enabled,maximum):
    value='1'+b.ue(2)+b.ue(0)+'1'+f'{1:08b}'
    value+='0'+b.ue(1)+b.ue(1)+b.ue(0)+'1'+b.ue(0)+'1'+b.ue(0)+b.ue(0)+'0'
    value+='101'+'00'+'1'+b.ue(2)+b.ue(2)+'00'+'0'+'0'
    value+=b.ue(1)+b.se(0)
    for list in range(2):
        value+='111'+'111'
        for ref in range(3):
            weight,offset=b.WEIGHTS[list][ref]
            value+=b.se(weight-2)+b.se(offset)
            for weight,offset in CHROMA[list][ref]:
                # Invert the signaled chroma midpoint correction. At denom1,
                # ChromaOffset=128+delta_offset-64*weight (7-58).
                value+=b.se(weight-2)+b.se(offset-128+64*weight)
    value+=b.ue(5+int(enabled)-maximum)+b.se(0)+b.ue(1)+'00000000'+'1'
    value+='0'*((-len(value))%8)
    return ((1<<9)|(5<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

@lru_cache(None)
def plane(view,future,component):
    # Different target/source view pixels prevent IV from passing by simply
    # copying the reference-view B picture instead of predicting own RPLs.
    offset=([0,19,71]if component==1 else[0,37,113])[view]
    sx,sy,bias=(13,7,29)if component==1 else(17,11,83)
    return tuple((x*sx+y*sy+bias+offset+(97 if component==1 else 194)*future)%256
        for y in range(16)for x in range(16))

@lru_cache(None)
def pcm(layer,bias):
    view={0:0,3:1,5:2}[layer];encoder,c=banks(0);encoder.decision(c['split0'],False)
    prefix=encoder.finish()
    pixels=bytes((x*3+y*5+17+bias)%256 for y in range(32)for x in range(32))
    pixels+=bytes(plane(view,bias!=0,1))+bytes(plane(view,bias!=0,2))
    encoder.low=0;encoder.range=510;encoder.width=9
    return prefix+pixels+encoder.finish()

def movie(enabled,intra,mode,index,weighted,maximum,explicit=False):
    units=[[b.header(layer,0,pcm(layer,0),2)for layer in [0,3,5]],
        [b.header(layer,2,pcm(layer,97),2,negative=[2])for layer in [0,3,5]],
        [b.header(0,1,pcm(0,0),2,negative=[1],positive=[1]),
         b.header(3,1,pcm(3,0)if intra else b.source(mode),2 if intra else 0,negative=[1],positive=[1]),
         weighted_header(b.target(index,maximum),enabled,maximum)if explicit else
         b.header(5,1,b.target(index,maximum),0,negative=[1],positive=[1],weighted=weighted,iv=enabled,maximum=maximum)]]
    stream=b''.join(b'\0\0\0\1'+n for n in b.parameters(enabled,weighted)+[n for au in units for n in au])
    return mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def clip(value):return min(255,max(0,value))

def interpolate(samples,x,y,mv):
    # MV is quarter-luma / eighth-chroma. Integer division is floor, including
    # negative vectors; edge extension precedes each independent filter tap.
    dx,px=divmod(mv[0],8);dy,py=divmod(mv[1],8)
    def at(xx,yy):return samples[min(15,max(0,yy))*16+min(15,max(0,xx))]
    return sum(cy*sum(cx*at(x+dx+i-1,y+dy+j-1)for i,cx in enumerate(FILTERS[px]))
        for j,cy in enumerate(FILTERS[py]))//64

@lru_cache(None)
def source_plane(intra,mode,component):
    past=plane(1,False,component);future=plane(1,True,component)
    if intra:return past
    result=[]
    for y in range(16):
        for x in range(16):
            a=interpolate(past,x,y,[0,4]);c=interpolate(future,x,y,[0,-8])
            result.append(clip((a+c+64)//128)if mode=='bi'else clip(((a if mode=='l0'else c)+32)//64))
    return tuple(result)

def target_plane(intra,mode,refs,motion,component,explicit=False):
    source=source_plane(intra,mode,component)
    l0=[plane(2,False,component),source,plane(2,True,component)]
    l1=[plane(2,True,component),plane(2,False,component),source]
    result=[]
    for y in range(16):
        for x in range(16):
            a=interpolate(l0[refs[0]],x,y,motion[0]);c=interpolate(l1[refs[1]],x,y,motion[1])
            if explicit:
                w0,o0=CHROMA[0][refs[0]][component-1];w1,o1=CHROMA[1][refs[1]][component-1]
                value=(w0*a+w1*c+128*(o0+o1+1))//256 # H.265 8-277, log2Wd=7
            else:value=(a+c+64)//128
            result.append(clip(value))
    return result

def rounding_witnesses():
    result=[]
    for component in [1,2]:
        for y in range(16):
            for x in range(16):
                a=interpolate(plane(1,False,component),x,y,[0,4]);c=interpolate(plane(1,True,component),x,y,[0,-8])
                correct=clip((a+c+64)//128)
                early=(clip((a+32)//64)+clip((c+32)//64)+1)//2
                if correct!=early:result.append(dict(component=component,x=x,y=y,correct=correct,early=early))
    assert result
    return result

def generate():
    files={};cases=[];golden=bytearray()
    for enabled in [False,True]:
        for weighted in [False,True]:
            for intra,mode in [(False,'bi'),(False,'l0'),(False,'l1'),(True,'bi')]:
                for index in range(6 if enabled else 5):
                    maximum=6 if index==5 else 5
                    for explicit in ([False,True]if weighted else[False]):
                        name='texture-iv-chroma-b-'+('on'if enabled else'off')+('-intra'if intra else'-'+mode)+('-weighted'if weighted else'')+('-cw'if explicit else'')+('-merge6'if maximum==6 else'')+f'-{index}.mp4'
                        files[name]=movie(enabled,intra,mode,index,weighted,maximum,explicit)
                        label,refs,motion,source_y,target_y=b.oracle(enabled,intra,mode,index,weighted)
                        offsets=[]
                        for planes in [[source_y,source_plane(intra,mode,1),source_plane(intra,mode,2)],
                                [target_y,target_plane(intra,mode,refs,motion,1,explicit),target_plane(intra,mode,refs,motion,2,explicit)]]:
                            indices=[]
                            for values in planes:
                                indices.append(len(golden)//2);golden.extend(struct.pack('<'+'H'*len(values),*values))
                            offsets.append(indices)
                        cases.append(dict(file=name,enabled=enabled,weighted=weighted,explicit_chroma=explicit,intra=intra,mode=mode,index=index,
                            maximum=maximum,label=label,references=refs,motion=motion,offsets=offsets))
    files['texture-iv-chroma-b-oracles.y16']=bytes(golden)
    files['texture-iv-chroma-b-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC and MP4, independent integer H.265 chroma filter/round/clip oracle, no private media, external codec, network or test-time generation.',
        chroma_specification='H.265 v8 8-241..8-261, 7-58, 8-277; BitDepthC=8 shift1=0/shift2=6, explicit denominator1; final rounding only.',
        filters=FILTERS,chroma_weights=CHROMA,rounding_witnesses=rounding_witnesses(),
        chroma_view_offsets=[[0,19,71],[0,37,113]],cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
