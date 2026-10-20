#!/usr/bin/env python3
"""Authored AVC mixed-depth pred_weight_table bitstreams and scalar oracles."""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux,configuration
from generate_avc_component_fractional import interpolate

def main():
    sources=json.loads((D/'avc-component-depths.json').read_text())['cases'];cases=[]
    variants=[
        ([0,0,0],[1,1,1],[1,-1,2],[True,True]),
        ([2,3,3],[3,-2,5],[-7,19,-13],[True,True]),
        ([7,7,7],[127,-128,0],[-128,127,-3],[True,True]),
        ([7,7,7],[128,128,128],[0,0,0],[False,False]),
        ([3,2,2],[-3,4,4],[127,0,0],[True,False]),
        ([2,0,0],[4,2,-1],[0,-128,127],[False,True]),
    ]
    for case in sources:
        first=case['frames'][0];config=configuration(case['luma_depth'],case['chroma_depth'],True)
        for variant,(denominators,weights,offsets,flags) in enumerate(variants):
            for mode,vector in [('skip',[0,0]),('hquarter',[1,0]),('vquarter',[0,-1])]:
                name=case['file'][:-4]+f'-weight{variant}-{mode}'
                configuration_file=name+'.avcc';(D/configuration_file).write_bytes(config)
                b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0)
                b.ue(denominators[0]);b.ue(denominators[1])
                b.u(int(flags[0]))
                if flags[0]:b.se(weights[0]);b.se(offsets[0])
                b.u(int(flags[1]))
                if flags[1]:
                    for p in [1,2]:b.se(weights[p]);b.se(offsets[p])
                b.u(0);b.se(0);b.ue(1)
                if mode=='skip':b.ue(1)
                else:b.ue(0);b.ue(0);b.se(vector[0]);b.se(vector[1]);b.ue(0)
                nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                file=name+'-1.packet';(D/file).write_bytes(packet);second=dict(packet=file)
                for component,(plane,width,depth) in enumerate([('y',16,case['luma_depth']),('cb',8,case['chroma_depth']),('cr',8,case['chroma_depth'])]):
                    raw=(D/first[plane]).read_bytes();samples=[int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)]
                    motion=vector[0] or vector[1];axis=int(vector[1]!=0)
                    samples=interpolate(samples,width,depth,motion,axis,component!=0)
                    denom=denominators[component];rounding=0 if denom==0 else 1<<(denom-1)
                    maximum=(1<<depth)-1
                    values=[min(maximum,max(0,((v*weights[component]+rounding)>>denom)+(offsets[component]<<(depth-8)))) for v in samples]
                    file=name+'-1.'+plane;(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));second[plane]=file
                data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(1,False,packet)],16,16,50)
                file=name+'.mp4';(D/file).write_bytes(data)
                cases.append(dict(case,file=file,configuration=configuration_file,weight_variant=variant,mode=mode,weights=weights,offsets=offsets,denominators=denominators,flags=flags,frames=[first,second],sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-weights.json').write_text(json.dumps(dict(cases=cases,provenance='Authored pred_weight_table and P-skip/P_L0_16x16 with independent scalar interpolation/weight/offset/rounding/clipping oracles. No external codec, production decoder, private media or network.'),indent=2)+'\n')
if __name__=='__main__':main()
