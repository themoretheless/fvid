#!/usr/bin/env python3
"""Authored reordered B_Bi_16x16, mixed-depth scalar bi-prediction oracles."""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux,configuration
from generate_avc_component_fractional import interpolate

def read(file):
    raw=(D/file).read_bytes()
    return [int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)]
def future_packet(frame,depths,poc):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(1,4);b.u(poc,4);b.u(0);b.se(0);b.ue(1);b.ue(25);b.align()
    for component,depth in zip(['y','cb','cr'],depths):
        for v in read(frame[component]):b.u(v,depth)
    nal=b.nal(0x41)
    return len(nal).to_bytes(4,'big')+nal

def main():
    cases=[];sources=json.loads((D/'avc-component-depths.json').read_text())['cases']
    variants=[
        ('average',0,4,[0,0,0],[[1]*3,[1]*3],[[0]*3,[0]*3],[[False]*2,[False]*2]),
        ('implicit-symmetric',2,4,[5]*3,[[32]*3,[32]*3],[[0]*3,[0]*3],[[False]*2,[False]*2]),
        ('implicit-asymmetric',2,6,[5]*3,[[43]*3,[21]*3],[[0]*3,[0]*3],[[False]*2,[False]*2]),
        ('explicit-mixed',1,4,[2,3,3],[[3,-2,5],[-1,7,0]],[[-7,19,-13],[13,-4,127]],[[True]*2,[True]*2]),
        ('explicit-extremes',1,4,[7]*3,[[127,-128,0],[-128,127,0]],[[-128,127,-3],[127,-128,0]],[[True]*2,[True]*2]),
        ('explicit-defaults',1,4,[7]*3,[[128]*3,[128]*3],[[0]*3,[0]*3],[[False]*2,[False]*2]),
        ('explicit-splitflags',1,4,[3,2,2],[[-3,4,4],[8,2,-1]],[[127,0,0],[0,-128,127]],[[True,False],[False,True]]),
    ]
    for case in sources:
        first,future=case['frames'];depths=[case['luma_depth'],case['chroma_depth'],case['chroma_depth']]
        for label,bipred,poc,denoms,weights,offsets,flags in variants:
            config=configuration(depths[0],depths[1],bipred=bipred,max_refs=2)
            for mode,vectors in [('integer',[[0,0],[0,0]]),('fractional',[[1,0],[0,-1]])]:
                name=case['file'][:-4]+f'-b-{label}-{mode}'
                config_file=name+'.avcc';(D/config_file).write_bytes(config)
                future_data=future_packet(future,depths,poc);file=name+'-future.packet';(D/file).write_bytes(future_data);future_frame=dict(future,packet=file)
                b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(2,4);b.u(2,4);b.u(1);b.u(0);b.u(0);b.u(0)
                if bipred==1:
                    b.ue(denoms[0]);b.ue(denoms[1])
                    for list_index in [0,1]:
                        b.u(int(flags[list_index][0]))
                        if flags[list_index][0]:b.se(weights[list_index][0]);b.se(offsets[list_index][0])
                        b.u(int(flags[list_index][1]))
                        if flags[list_index][1]:
                            for p in [1,2]:b.se(weights[list_index][p]);b.se(offsets[list_index][p])
                b.se(0);b.ue(1);b.ue(0);b.ue(3)
                for vector in vectors:b.se(vector[0]);b.se(vector[1])
                b.ue(0);nal=b.nal(0x01);packet=len(nal).to_bytes(4,'big')+nal
                file=name+'-b.packet';(D/file).write_bytes(packet);middle=dict(packet=file)
                for p,(plane,width,depth) in enumerate(zip(['y','cb','cr'],[16,8,8],depths)):
                    predictions=[]
                    for frame,vector in zip([first,future],vectors):
                        predictions.append(interpolate(read(frame[plane]),width,depth,vector[0] or vector[1],int(vector[1]!=0),p!=0))
                    denom=denoms[p];maximum=(1<<depth)-1
                    offset=((offsets[0][p]+offsets[1][p])*(1<<(depth-8))+1)//2
                    values=[min(maximum,max(0,((a*weights[0][p]+c*weights[1][p]+(1<<denom))>>(denom+1))+offset)) for a,c in zip(*predictions)]
                    file=name+'-b.'+plane;(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));middle[plane]=file
                data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(2,False,future_data),(1,False,packet)],16,16,50)
                file=name+'.mp4';(D/file).write_bytes(data)
                cases.append(dict(case,file=file,configuration=config_file,frames=[first,middle,future_frame],decode_order=[0,2,1],bipred=bipred,variant=label,mode=mode,sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-bipred.json').write_text(json.dumps(dict(cases=cases,provenance='Authored IDR/future I_PCM/B_Bi_16x16, independent scalar interpolation and uni/implicit/explicit bipred integer arithmetic. Presentation order differs from decode order. No external codec, private media or network.'),indent=2)+'\n')
if __name__=='__main__':main()
