#!/usr/bin/env python3
"""Authored axial quarter-luma/eighth-chroma motion, independent scalar oracles.

H.264 8.4.2.2: six-tap half positions, rounded averages at quarter positions,
and bilinear chroma. No FVid decoder, external codec, private media or network.
"""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux

def interpolate(samples,width,depth,motion,axis,chroma):
    height=len(samples)//width;maximum=(1<<depth)-1;result=[]
    def at(x,y):return samples[min(height-1,max(0,y))*width+min(width-1,max(0,x))]
    denominator=8 if chroma else 4
    whole,fraction=divmod(motion,denominator)
    for y in range(height):
        for x in range(width):
            def value(offset):return at(x+whole+offset,y) if axis==0 else at(x,y+whole+offset)
            if chroma:
                output=((8-fraction)*value(0)+fraction*value(1)+4)//8
            elif fraction==0:output=value(0)
            else:
                half=min(maximum,max(0,(sum(a*value(b) for a,b in zip([1,-5,20,20,-5,1],[-2,-1,0,1,2,3]))+16)//32))
                output=half if fraction==2 else (half+value(0 if fraction==1 else 1)+1)//2
            result.append(output)
    return result

def main():
    sources=json.loads((D/'avc-component-depths.json').read_text())['cases'];cases=[]
    for case in sources:
        first=case['frames'][0];config=(D/case['configuration']).read_bytes()
        for axis in [0,1]:
            for motion in [-4,-3,-2,-1,1,2,3,4]:
                vector=[motion if axis==0 else 0,motion if axis==1 else 0]
                name=case['file'][:-4]+f'-motion-{vector[0]}-{vector[1]}'
                b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4)
                b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
                b.ue(0);b.ue(0);b.se(vector[0]);b.se(vector[1]);b.ue(0)
                nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                file=name+'-1.packet';(D/file).write_bytes(packet);second=dict(packet=file)
                for component,width,depth in [('y',16,case['luma_depth']),('cb',8,case['chroma_depth']),('cr',8,case['chroma_depth'])]:
                    raw=(D/first[component]).read_bytes();samples=[int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)]
                    values=interpolate(samples,width,depth,motion,axis,component!='y')
                    file=name+'-1.'+component;(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));second[component]=file
                data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(1,False,packet)],16,16,50)
                file=name+'.mp4';(D/file).write_bytes(data)
                cases.append(dict(case,file=file,motion=vector,frames=[first,second],sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-fractional.json').write_text(json.dumps(dict(cases=cases,provenance='Independent scalar H.264 axial motion interpolation on authored PCM arrays; three luma fractional phases, all seven chroma phases, both signs/axes, edge extension and clipping. No private media or external codec.'),indent=2)+'\n')
if __name__=='__main__':main()
