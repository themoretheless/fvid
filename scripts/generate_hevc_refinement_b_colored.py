"""Owned colored B refinement with separate scalar interpolation/weight oracle."""
import json,struct
from functools import lru_cache
import generate_hevc_refinement_b as r
import generate_hevc_texture_iv_chroma_b as c
from generate_hevc_texture_iv_b import WEIGHTS
from generate_hevc_cqt import OUT
LUMA=[(0,0,0,64,0,0,0,0),(-1,4,-10,58,17,-5,1,0),(-1,4,-11,40,40,-11,4,-1),(0,1,-5,17,58,-10,4,-1)]

def direction(mode,x):return -1 if mode.startswith('amp')and x>=int(mode[3:])else 1

def pattern(x,y,future):return(3*min(31,max(0,x))+5*min(31,max(0,y))+17+97*future)%256

@lru_cache(None)
def source(mode,component):
    if component==0:
        def at(x,y):
            sign=direction(mode,x);a=pattern(x,y+sign,False);b=pattern(x,y-2*sign,True)
            return pattern(x,y,False)if mode=='intra'else a if mode=='l0'else b if mode=='l1'else(a+b+1)//2
        return tuple(at(x,y)for y in range(32)for x in range(32))
    if mode=='intra':return c.plane(0,False,component)
    values=[]
    for y in range(16):
        for x in range(16):
            sign=direction(mode,x*2)
            a=c.interpolate(c.plane(0,False,component),x,y,[0,4*sign])
            b=c.interpolate(c.plane(0,True,component),x,y,[0,-8*sign])
            values.append(c.clip(((a if mode=='l0'else b)+32)//64)if mode in ['l0','l1']else c.clip((a+b+64)//128))
    return tuple(values)

def target(mode,disparity,weights,component):
    side=32 if component==0 else 16;values=[];witnesses=0
    for y in range(side):
        for x in range(side):
            if mode.startswith('amp'):
                luma_x=x if component==0 else x*2
                lookup=min(31,max(0,(((luma_x//8*8+4+((disparity+2)>>2))>>3)<<3)))
                sign=direction(mode,lookup);refs=[0,0]
                if component==0:a=64*pattern(x,y+sign,False);b=64*pattern(x,y-2*sign,True)
                else:
                    a=c.interpolate(c.plane(2,False,component),x,y,[0,4*sign])
                    b=c.interpolate(c.plane(2,True,component),x,y,[0,-8*sign])
            else:
                refs=[1,2]
                plane=source(mode,component)
                if component==0:
                    dx,phase=divmod(disparity,4)
                    a=sum(w*plane[y*32+min(31,max(0,x+dx+i-3))]for i,w in enumerate(LUMA[phase]))
                else:a=c.interpolate(plane,x,y,[disparity,0])
                b=a
            chosen=(WEIGHTS if component==0 else c.CHROMA)
            active=weights!='none'if component==0 else weights=='color'
            pair=[chosen[l][refs[l]]if component==0 else chosen[l][refs[l]][component-1] for l in range(2)]if active else[(2,0),(2,0)]
            (w0,o0),(w1,o1)=pair
            value=c.clip((w0*a+w1*b+128*(o0+o1+1))//256)
            early=c.clip((w0*c.clip((a+32)//64)+w1*c.clip((b+32)//64)+2*(o0+o1+1))//4)
            witnesses+=value!=early;values.append(value)
    return values,witnesses

def generate():
    files={};cases=[];golden=bytearray();witnesses=0
    for enabled in [False,True]:
        for weights in ['none','luma','color']:
            for mode in ['intra','bi','l0','l1','amp8','amp24']:
                for scale in [0,2,-2,4,-4,6,-6,8,-8,128,-128]:
                    name=f'texture-refinement-b-colored-{int(enabled)}-{weights}-{mode}-{scale}.mp4'
                    files[name]=r.movie(enabled,weights!='none',mode,scale,precision=2,colored=True,explicit=weights=='color')
                    disparity=(scale*254+256)>>9 if enabled else 0
                    offsets=[]
                    for owner in ['source','target']:
                        offsets.append(len(golden)//2)
                        for component in range(3):
                            if owner=='source':values=source(mode,component)
                            else:values,count=target(mode,disparity,weights,component);witnesses+=count
                            golden.extend(struct.pack('<'+str(len(values))+'H',*values))
                    cases.append(dict(file=name,enabled=enabled,weights=weights,mode=mode,scale=scale,disparity=disparity,offsets=offsets))
    files['texture-refinement-b-colored.y16']=bytes(golden)
    files['texture-refinement-b-colored.json']=(json.dumps(dict(
        provenance='Owned mixed-role PCM/CABAC and MP4; separate scalar 8-/4-tap interpolation and bi-weight oracle. No external codec, private media or test-time generation.',
        precision=2,rounding_witnesses=witnesses,cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
