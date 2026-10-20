#!/usr/bin/env python3
"""Authored mixed-depth CAVLC AC impulses; independent inverse-transform matrix."""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux
from generate_avc_component_residual import CHROMA

SCAN=[0,1,4,8,5,2,3,6,9,12,13,10,7,11,14,15]
NORMAL=[[10,16,13],[11,18,14],[13,20,16],[14,23,18],[16,25,20],[18,29,23]]
BASIS=[[2,2,2,1],[2,1,-2,-2],[2,-1,-2,2],[2,-2,2,-1]]
ZEROS=[(1,1),(3,3),(2,3),(3,4),(2,4),(3,5),(2,5),(3,6),(2,6),(3,7),(2,7),(3,8),(2,8),(3,9),(2,9),(1,9)]
def block(qp,depth,scan_index,sign,chroma):
    q=(CHROMA[qp-30] if chroma and qp>=30 else qp)+6*(depth-8)
    row,col=divmod(SCAN[scan_index],4)
    category=0 if row%2==col%2==0 else 1 if row%2==col%2==1 else 2
    scaled=sign*NORMAL[q%6][category]*(1<<(q//6))
    assert scaled%4==0 # Matrix formulation is exact; no intermediate half-shift rounding.
    return [(BASIS[y][row]*scaled*BASIS[x][col]//4+32)//64 for y in range(4) for x in range(4)]
def coefficient(b,sign,zeros):
    b.u(1,2);b.u(int(sign<0));code,width=ZEROS[zeros];b.u(code,width)

def main():
    cases=[];sources=json.loads((D/'avc-component-depths.json').read_text())['cases']
    for case in sources:
        first=case['frames'][0];config=(D/case['configuration']).read_bytes()
        positions=[(s,24+(s+case['luma_depth']+3*case['chroma_depth'])%6) for s in range(1,16)]
        positions += [(s,q) for s in [1,4,5] for q in [30,51]]
        for scan_index,qp in positions:
            for mode in ['luma','chroma','combined']:
                for polarity in [1,-1]:
                    name=case['file'][:-4]+f'-ac{scan_index}-{mode}-qp{qp}-sign{polarity}'
                    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4)
                    b.u(0);b.u(0);b.u(0);b.se(qp-26);b.ue(1)
                    b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue({'luma':11,'chroma':6,'combined':12}[mode]);b.se(0)
                    if mode!='chroma':
                        for index in range(16):coefficient(b,polarity*(1 if index%2==0 else -1),scan_index)
                    if mode!='luma':
                        for _ in range(2):b.u(1,2) # Zero 2x2 chroma DC.
                        for component in range(2):
                            for index in range(4):coefficient(b,polarity*(1 if (index+component)%2==0 else -1),scan_index-1)
                    nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                    file=name+'-1.packet';(D/file).write_bytes(packet);second=dict(packet=file);residual_values=[]
                    for component,width,depth in [('y',16,case['luma_depth']),('cb',8,case['chroma_depth']),('cr',8,case['chroma_depth'])]:
                        raw=(D/first[component]).read_bytes();source=[int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)];values=[];deltas=[]
                        coded=(component=='y' and mode!='chroma') or (component!='y' and mode!='luma')
                        patterns={s:block(qp,depth,scan_index,s,component!='y') for s in [1,-1]}
                        for i,value in enumerate(source):
                            x,y=i%width,i//width
                            sign=polarity*(1 if ((x//4)+int(component=='cr'))%2==0 else -1)
                            delta=patterns[sign][(y%4)*4+x%4] if coded else 0
                            deltas.append(delta);values.append(min((1<<depth)-1,max(0,value+delta)))
                        file=name+'-1.'+component;(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));second[component]=file;residual_values.append(sorted(set(deltas)))
                    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(2,4);b.u(4,4)
                    b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(1)
                    nal=b.nal(0x41);echo=len(nal).to_bytes(4,'big')+nal
                    file=name+'-2.packet';(D/file).write_bytes(echo);third=dict(second,packet=file)
                    data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(1,False,packet),(2,False,echo)],16,16,50)
                    file=name+'.mp4';(D/file).write_bytes(data)
                    cases.append(dict(case,file=file,frames=[first,second,third],scan_index=scan_index,qp=qp,residual_mode=mode,polarity=polarity,residual_values=residual_values,sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-ac.json').write_text(json.dumps(dict(cases=cases,provenance='All 15 AC basis positions, authored CAVLC levels and independent integer inverse-transform matrix. Flat-16 scaling makes intermediate half shifts exact. Reconstructed P-skip reference echo, both signs, luma/chroma/all, source-depth clipping. No external codec, private media, production decoder or network.'),indent=2)+'\n')
if __name__=='__main__':main()
