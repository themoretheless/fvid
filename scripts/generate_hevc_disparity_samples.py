"""Owned I-300..I-312 component oracles and short coded depth source, offline."""
import json, math, struct
from fractions import Fraction
from generate_hevc_wedge_tables import OUT
from generate_hevc_depth_view_scaling import generate_geometry
from generate_hevc_disparity_oracles import oracle

VALUES=[-3,-1,2,4]
DEPTH=[(x*7+y*11+37)%256 for y in range(32) for x in range(32)]
def expected(origin,block,vector,part):
    w,h=block
    rx,ry=[o+math.floor(Fraction(v,4)+Fraction(1,2)) for o,v in zip(origin,vector)]
    def sample(x,y):return DEPTH[min(31,max(0,ry+y))*32+min(31,max(0,rx+x))]
    horizontal=False
    if part==2:
        if w%8 or h%8:horizontal=bool(h%8)
        else:horizontal=(sample(0,0)<sample(w-1,h-1))==(sample(w-1,0)<sample(0,h-1))
    sw,sh=(w,h)if part==0 else((1,1)if part==1 else((8,4)if horizontal else(4,8)))
    result=[]
    # Evaluate each output location independently, without a fill/block loop.
    for y in range(h):
        for x in range(w):
            x0,y0=(x//sw)*sw,(y//sh)*sh
            corners=[sample(xx,yy)for xx in [x0,x0+sw-1]for yy in [y0,y0+sh-1]]
            value=max(corners)
            result.append(value if part==1 else oracle(VALUES,2,8,value)[0])
    return horizontal,result

def generate():
    cases=[];payload=bytearray()
    for block in [[8,8],[16,8],[8,16],[32,32],[8,4],[4,8]]:
        for origin in [[0,0],[5,9],[31,31]]:
            for vector in [[0,0],[-3,2],[-2,-2],[1,3],[125,-129],[-2**63,2**63-1],[2**63-1,-2**63]]:
                for part in range(3):
                    horizontal,values=expected(origin,block,vector,part)
                    offset=len(payload)//8
                    payload.extend(struct.pack('<'+'q'*len(values),*values))
                    cases.append(dict(block=block,origin=origin,vector=vector,part=part,horizontal=horizontal,offset=offset))
    movie=generate_geometry()['depth-view-temporal-scaling-off.mp4']
    return {'disparity-depth-source.mp4':movie,'disparity-sample-oracles.i64':bytes(payload),
        'disparity-sample-oracles.json':(json.dumps(dict(
            provenance='Owned PCM/CABAC depth movie and independent per-output-sample Python Fraction oracle for I-300..I-312. Extreme public arithmetic cases are component ranges, not coded MV acceptance. No FFmpeg, private source or network.',
            dimensions=[32,32],values=VALUES,precision=2,depth=8,cases=cases),indent=2)+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
