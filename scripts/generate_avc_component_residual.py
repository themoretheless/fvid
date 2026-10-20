#!/usr/bin/env python3
"""Authored mixed-depth CAVLC inter DC impulses and closed-form pixel oracles."""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux

# Normative chroma QP remapping and DC normalization factors, not decoder output.
CHROMA=[29,30,31,32,32,33,34,34,35,35,36,36,37,37,37,38,38,38,39,39,39,39]
NORMAL=[10,11,13,14,16,18]
def increment(qp,depth,sign,chroma):
    mapped=qp if not chroma or qp<30 else CHROMA[qp-30]
    q=mapped+6*(depth-8)
    scaled=sign*NORMAL[q%6]*(1<<(q//6))
    if chroma:scaled//=2
    return (scaled+32)//64

def main():
    cases=[];sources=json.loads((D/'avc-component-depths.json').read_text())['cases']
    for case in sources:
        first=case['frames'][0];config=(D/case['configuration']).read_bytes()
        for qp in [24,25,26,27,28,29,30,51]:
            for mode in ['luma','chroma','combined']:
                for polarity in [1,-1]:
                    name=case['file'][:-4]+f'-residual-{mode}-qp{qp}-sign{polarity}'
                    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4)
                    b.u(0);b.u(0);b.u(0);b.se(qp-26);b.ue(1)
                    b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue({'luma':11,'chroma':1,'combined':19}[mode]);b.se(0)
                    if mode!='chroma':
                        for block in range(16):
                            sign=polarity*(1 if block%2==0 else -1)
                            b.u(1,2);b.u(int(sign<0));b.u(1)
                    if mode!='luma':
                        for component in [0,1]:
                            sign=polarity*(1 if component==0 else -1)
                            b.u(1);b.u(int(sign<0));b.u(1)
                    nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                    file=name+'-1.packet';(D/file).write_bytes(packet);second=dict(packet=file);deltas=[]
                    for component,width,depth in [('y',16,case['luma_depth']),('cb',8,case['chroma_depth']),('cr',8,case['chroma_depth'])]:
                        raw=(D/first[component]).read_bytes();source=[int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)];values=[];residual=[]
                        for i,value in enumerate(source):
                            coded=(component=='y' and mode!='chroma') or (component!='y' and mode!='luma')
                            sign=polarity*((1 if (i%width)//4%2==0 else -1) if component=='y' else (1 if component=='cb' else -1))
                            delta=increment(qp,depth,sign,component!='y') if coded else 0
                            residual.append(delta);values.append(min((1<<depth)-1,max(0,value+delta)))
                        file=name+'-1.'+component;(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));second[component]=file;deltas.append(sorted(set(residual)))
                    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(2,4);b.u(4,4)
                    b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(1)
                    nal=b.nal(0x41);echo=len(nal).to_bytes(4,'big')+nal
                    file=name+'-2.packet';(D/file).write_bytes(echo);third=dict(second,packet=file)
                    data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(1,False,packet),(2,False,echo)],16,16,50)
                    file=name+'.mp4';(D/file).write_bytes(data)
                    cases.append(dict(case,file=file,frames=[first,second,third],qp=qp,residual_mode=mode,polarity=polarity,residual_values=deltas,sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-residual.json').write_text(json.dumps(dict(cases=cases,provenance='Authored nonzero signed CAVLC block-DC/chroma-DC impulses. Third P-skip verifies retained reconstructed reference samples. Independent closed-form inverse DC scaling, chroma QP remapping, rounding and per-plane clipping. No external codec, private media, production decoder or network.'),indent=2)+'\n')
if __name__=='__main__':main()
