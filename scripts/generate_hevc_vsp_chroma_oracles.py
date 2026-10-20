"""Owned nonconstant PCM chroma movie and independent 4-tap VSP oracle."""
import json,math,struct
from fractions import Fraction
from generate_hevc_wedge_tables import OUT
import generate_hevc_vsp_oracles as luma
from generate_hevc_inter_view_scaling import generate_case
# H.265 8-241..8-247 plus integer identity; no production decoder imports.
FILTERS=[[0,64,0,0],[-2,58,10,-2],[-4,54,16,-2],[-6,46,28,-4],[-4,36,36,-4],[-4,28,46,-6],[-2,16,54,-4],[-2,10,58,-2]]
def expected(rect,disparities,component):
    ox,oy,w,h=rect;result=[]
    for y in range(h//2):
        for x in range(w//2):
            mv=disparities[(y*2)*w+x*2]
            integer=math.floor(Fraction(mv,8));phase=mv%8
            total=sum(weight*((min(15,max(0,ox//2+x+integer+k-1))*13+(oy//2+y)*7+29+component*83)%256)for k,weight in enumerate(FILTERS[phase]))
            result.append(min(255,max(0,math.floor(Fraction(total+32,64)))))
    return result

def generate():
    old=json.loads(luma.generate()['vsp-luma-oracles.json'])['cases']
    cases=list(old)
    for case in old[:24]:
        c=dict(case,values=[0,0,0,0])
        c['horizontal'],c['disparities'],_=luma.expected(c['rect'],c['vector'],c['values'])
        cases.append(c)
    data=bytearray();output=[]
    for case in cases:
        c=dict(case,offset=len(data)//2)
        for component in range(2):
            pixels=expected(c['rect'],c['disparities'],component)
            data.extend(struct.pack('<'+'H'*len(pixels),*pixels))
        output.append(c)
    movie=generate_case('',(100,102,108),4,all_intra=True,chroma_pattern=True)['inter-view-temporal-scaling-on.mp4']
    return {'vsp-chroma-source.mp4':movie,'vsp-chroma-oracles.y16':bytes(data),
        'vsp-chroma-oracles.json':(json.dumps(dict(provenance='Owned PCM nonconstant Cb/Cr and independent Fraction/four-tap chroma oracle for 4:2:0 VSP component pipeline. All eight horizontal eighth-sample phases, no vertical VSP MV. No private source, FFmpeg, network or test-time generation. Does not signal coded VSP tool acceptance.',cases=output),indent=2)+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
