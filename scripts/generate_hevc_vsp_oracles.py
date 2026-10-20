"""Independent scalar luma oracle for owned VSP component reconstruction."""
import json,math,struct
from fractions import Fraction
from generate_hevc_wedge_tables import OUT
from generate_hevc_disparity_oracles import oracle
FILTERS=[[0,0,0,64,0,0,0,0],[-1,4,-10,58,17,-5,1,0],[-1,4,-11,40,40,-11,4,-1],[0,1,-5,17,58,-10,4,-1]]
def expected(rect,vector,values):
    ox,oy,w,h=rect
    rx,ry=[o+math.floor(Fraction(v,4)+Fraction(1,2))for o,v in zip([ox,oy],vector)]
    def depth(x,y):return (min(31,max(0,rx+x))*7+min(31,max(0,ry+y))*11+37)%256
    horizontal=(bool(h%8)if w%8 or h%8 else(depth(0,0)<depth(w-1,h-1))==(depth(w-1,0)<depth(0,h-1)))
    sw,sh=(8,4)if horizontal else(4,8)
    output=[];disparities=[]
    for y in range(h):
        for x in range(w):
            x0,y0=x//sw*sw,y//sh*sh
            d=max(depth(xx,yy)for xx in [x0,x0+sw-1]for yy in [y0,y0+sh-1])
            mv=oracle(values,2,8,d)[0];disparities.append(mv)
            integer=math.floor(Fraction(mv,4));phase=mv%4
            total=sum(weight*((min(31,max(0,ox+x+integer+k-3))*3+(oy+y)*5+17)%256)for k,weight in enumerate(FILTERS[phase]))
            output.append(min(255,max(0,math.floor(Fraction(total+32,64)))))
    return horizontal,disparities,output

def generate():
    cases=[];data=bytearray()
    for values in [[8,-1,0,0],[-8,0,0,0]]:
        for rect in [[0,0,8,8],[8,8,16,8],[16,0,8,16],[0,0,32,32]]:
            for vector in [[0,0],[-3,2],[125,-129]]:
                horizontal,disparities,pixels=expected(rect,vector,values)
                for selected_list in [0,1]:
                    offset=len(data)//2;data.extend(struct.pack('<'+'H'*len(pixels),*pixels))
                    cases.append(dict(values=values,rect=rect,vector=vector,horizontal=horizontal,list=selected_list,offset=offset,disparities=disparities))
    return {'vsp-luma-oracles.y16':bytes(data),'vsp-luma-oracles.json':(json.dumps(dict(
        provenance='Independent per-pixel exact rational rounding and normative eight-tap scalar luma filters. Uses owned disparity-depth-source and inter-view-temporal-scaling-on movies. Manually supplied VSP inputs, not coded VSP admission. No private media, external codec, network or test-time generation.',cases=cases),indent=2)+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
