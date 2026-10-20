"""Independent priority/rule oracle for I.8.5.5.1 metadata; fully offline."""
import json
from generate_hevc_wedge_tables import OUT
GEOMETRIES=[[16,16,8,2,5],[16,32,8,2,5],[8,8,8,6,6],[0,0,8,2,5]]
# Neighbour types: absent, direct L0, direct L1, temporal-only, skip-IV, inactive.
def expected(geometry,left,top,merge,temporal,default):
    if temporal:return [5,90,91]
    x,y,size,mlog,ctu=geometry
    eligible=[]
    for index,(kind,point)in enumerate(zip([left,top],[[x-1,y+size-1],[x+size-1,y-1]])):
        if kind==0 or min(point)<0:continue
        if merge and x//2**mlog==point[0]//2**mlog and y//2**mlog==point[1]//2**mlog:continue
        eligible.append((index,kind,point))
    direct={1:[1,1,2],2:[2,3,4]}
    for _,kind,_ in eligible:
        if kind in direct:return direct[kind]
    for index,kind,point in eligible:
        if kind==4 and(index==0 or point[1]//2**ctu==y//2**ctu):return [3,7,8]
    return [0,0,0]if default else None

def generate():
    cases=[]
    for g,geometry in enumerate(GEOMETRIES):
        for left in range(6):
            for top in range(6):
                for merge in [False,True]:
                    for temporal in [False,True]:
                        for default in [False,True]:
                            cases.append([g,left,top,merge,temporal,default,expected(geometry,left,top,merge,temporal,default)])
    return {'disparity-derivation-oracles.json':(json.dumps(dict(provenance='Independent outcome-priority model for spatial disparity metadata, including CTU and parallel merge boundaries. 1152 cases. Not coded IV/DI/VSP acceptance. No FFmpeg, private media, network or test-time generation.',geometries=GEOMETRIES,cases=cases),separators=(',',':'))+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
