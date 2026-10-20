#!/usr/bin/env python3
"""Authored mixed-depth CAVLC transform8; independent exact rational matrix."""
import hashlib,json
from generate_avc_component_depths import D,Writer,mux,configuration
from generate_avc_component_ac import ZEROS

SCAN=[0,1,8,16,9,2,3,10,17,24,32,25,18,11,4,5,12,19,26,33,40,48,41,34,27,20,13,6,7,14,21,28,35,42,49,56,57,50,43,36,29,22,15,23,30,37,44,51,58,59,52,45,38,31,39,46,53,60,61,54,47,55,62,63]
NORMAL=[[20,18,32,19,25,24],[22,19,35,21,28,26],[26,23,42,24,33,31],[28,25,45,26,35,33],[32,28,51,30,40,38],[36,32,58,34,46,43]]
CATEGORIES=[[0,3,4,3],[3,1,5,1],[4,5,2,5],[3,1,5,1]]
BASIS=[[8,12,8,10,8,6,4,3],[8,10,4,-3,-8,-12,-8,-6],[8,6,-4,-12,-8,3,8,10],[8,3,-8,-6,8,10,-4,-12],[8,-3,-8,6,8,-10,-4,12],[8,-6,-4,12,-8,-3,8,-10],[8,-10,4,3,-8,12,-8,6],[8,-12,8,-10,8,-6,4,-3]]
def pattern(scan,qp,depth,sign):
    row,col=divmod(SCAN[scan],8);q=qp+6*(depth-8)
    scaled=sign*2*16*NORMAL[q%6][CATEGORIES[row%4][col%4]]*(1<<(q//6-6))
    assert scaled%64==0
    return [(BASIS[y][row]*scaled*BASIS[x][col]//64+32)//64 for y in range(8) for x in range(8)]

def main():
    cases=[];sources=json.loads((D/'avc-component-depths.json').read_text())['cases']
    for case in sources:
        first=case['frames'][0];depth=case['luma_depth'];config=configuration(depth,case['chroma_depth'],transform8=True)
        for scan in range(64):
            qp=46+(scan+depth+3*case['chroma_depth'])%6
            for polarity in [1,-1]:
                name=case['file'][:-4]+f'-transform8-scan{scan}-qp{qp}-sign{polarity}'
                config_file=name+'.avcc';(D/config_file).write_bytes(config)
                b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4)
                b.u(0);b.u(0);b.u(0);b.se(qp-26);b.ue(1)
                b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(11);b.u(1);b.se(0)
                for group in range(4):
                    sign=polarity*(1 if group%2==0 else -1)
                    for stream in range(4):
                        if stream!=scan%4:b.u(1)
                        else:
                            b.u(5,6) # TotalCoeff=1, TrailingOnes=0, nC<2.
                            b.u(1,1 if sign>0 else 2) # First adjusted level code: +2/-2.
                            code,width=ZEROS[scan//4];b.u(code,width)
                nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal
                file=name+'-1.packet';(D/file).write_bytes(packet);second=dict(first,packet=file)
                raw=(D/first['y']).read_bytes();source=[int.from_bytes(raw[i:i+2],'little') for i in range(0,len(raw),2)]
                patterns={s:pattern(scan,qp,depth,s) for s in [1,-1]};values=[];deltas=[]
                for i,v in enumerate(source):
                    x,y=i%16,i//16;sign=polarity*(1 if x//8==0 else -1)
                    delta=patterns[sign][(y%8)*8+x%8];deltas.append(delta);values.append(min((1<<depth)-1,max(0,v+delta)))
                file=name+'-1.y';(D/file).write_bytes(b''.join(v.to_bytes(2,'little') for v in values));second['y']=file
                b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(2,4);b.u(4,4)
                b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1);b.ue(1)
                nal=b.nal(0x41);echo=len(nal).to_bytes(4,'big')+nal
                file=name+'-2.packet';(D/file).write_bytes(echo);third=dict(second,packet=file)
                data=mux(config,[(0,True,(D/first['packet']).read_bytes()),(1,False,packet),(2,False,echo)],16,16,50)
                file=name+'.mp4';(D/file).write_bytes(data)
                cases.append(dict(case,file=file,configuration=config_file,frames=[first,second,third],scan_index=scan,qp=qp,polarity=polarity,residual_values=sorted(set(deltas)),sha256=hashlib.sha256(data).hexdigest()))
    (D/'avc-component-transform8.json').write_text(json.dumps(dict(cases=cases,provenance='All 64 transform8 basis positions, authored CAVLC interleaving and signed level two. Separate exact rational 8x8 matrix, flat scaling, distributed QP 46..51, mixed depths, clipping and P-skip reference echo. No private media, production decoder, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
