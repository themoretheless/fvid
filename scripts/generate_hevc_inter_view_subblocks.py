"""Owned textured AMP motion sources and scalar IV sub-block pixel oracles."""
import json,math,struct
from fractions import Fraction
from generate_hevc_wedge_tables import OUT
from generate_hevc_cqt import exact_texture_motion
from generate_hevc_vsp_oracles import FILTERS as LUMA
from generate_hevc_vsp_chroma_oracles import FILTERS as CHROMA

def blocks(rect,log):
    x,y,w,h=rect;m=2**log
    sw,sh=(w,h)if w%m or h%m else(m,m)
    return sw,sh,[[x+i*sw,y+j*sh,sw,sh]for j in range(h//sh)for i in range(w//sw)]
def vector_at(block,disparity,magnitude):
    # Compute reference alignment arithmetically, without production helpers.
    x,y,w,h=block
    full=y+h//2+math.floor(Fraction(disparity[1],4)+Fraction(1,2))
    source_y=min(31,max(0,math.floor(Fraction(full,8))*8))
    return magnitude if source_y<8 else-magnitude
def oracle(rect,log,disparity,magnitude):
    x,y,w,h=rect;sw,sh,parts=blocks(rect,log)
    default=[x+(w//sw//2)*sw,y+(h//sh//2)*sh,sw,sh]
    motion=[vector_at(p,disparity,magnitude)for p in parts]
    default_motion=vector_at(default,disparity,magnitude)
    planes=[]
    for component in range(3):
        shift=bool(component);side=16 if component else 32
        divisor=8 if component else 4;coefficients=CHROMA if component else LUMA
        start=-1 if component else-3;out=[]
        for yy in range(h>>shift):
            for xx in range(w>>shift):
                row=(yy<<shift)//sh;col=(xx<<shift)//sw;mv=motion[row*(w//sw)+col]
                integer=math.floor(Fraction(mv,divisor));phase=mv%divisor
                sx=(x>>shift)+xx;sy=(y>>shift)+yy
                def sample(oy):
                    sy_clipped=min(side-1,max(0,oy))
                    return (sx*13+sy_clipped*7+29+(component-1)*83)%256 if component else(sx*3+sy_clipped*5+17)%256
                total=sum(weight*sample(sy+integer+k+start)for k,weight in enumerate(coefficients[phase]))
                out.append(min(255,max(0,math.floor(Fraction(total+32,64)))))
        planes.append(out)
    return [sw,sh],default_motion,motion,planes

def generate():
    files={};data=bytearray();cases=[]
    for magnitude in [1,4]:
        files[f'iv-subblocks-source-{magnitude}.mp4']=next(iter(exact_texture_motion(magnitude,active=True,enabled=False,cqt=False,textured_source=True).values()))
        for rect in [[0,0,32,32],[0,0,16,32],[0,0,32,8],[0,0,8,32],[4,0,12,16]]:
            for log in range(3,7):
                for disparity in [[0,0],[0,13],[0,14],[0,-19],[2**63-1,-2**63]]:
                    size,default,motion,planes=oracle(rect,log,disparity,magnitude)
                    offsets=[]
                    for pixels in planes:
                        offsets.append(len(data)//2);data.extend(struct.pack('<'+'H'*len(pixels),*pixels))
                    cases.append(dict(magnitude=magnitude,rect=rect,log=log,disparity=disparity,size=size,default=default,motion=motion,offsets=offsets))
    files['iv-subblocks-oracles.y16']=bytes(data)
    files['iv-subblocks-oracles.json']=(json.dumps(dict(provenance='Owned PCM/CABAC textured AMP sources and independent Fraction scalar 8/4-tap Y/Cb/Cr oracles. Explicit IV inputs, no coded IV tool admission. No private media, external codec, network or test-time generation.',cases=cases),separators=(',',':'))+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
