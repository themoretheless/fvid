#!/usr/bin/env python3
"""Owned two-frame Y4M reference pictures and rational H.8.1.4.4 oracle.
Uses saved owned PPS LUT oracles, never a production sampler or external codec.
"""
import json
import struct
from fractions import Fraction as F
from math import floor
from pathlib import Path
D = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'


def rounded(v):
    return floor(v+F(1,2))


def mapping(table, c, values):
    ny,nc,_=table['dimensions']
    iy=values[0]//((1 << table['input'][0])//ny)
    cb=int(values[1]>=table['thresholds'][0]) if nc==2 else 0
    cr=int(values[2]>=table['thresholds'][1]) if nc==2 else 0
    matrix=table['coefficients'][(iy*nc+cb)*nc+cr]
    value=rounded(sum(F(matrix[j][c]*v,1 << (10+table['input'][0]-table['output'][0])) for j,v in enumerate(values)))+matrix[3][c]
    return max(0,min((1 << table['output'][c!=0])-1,value))


def picture(table, fmt, w, h, frame):
    sx,sy={1:(2,2),2:(2,1),3:(1,1)}[fmt]; cw,ch=w//sx,h//sy
    sizes=[(w,h),(cw,ch),(cw,ch)]
    planes=[]
    for c,(width,height) in enumerate(sizes):
        maximum=(1 << table['input'][c!=0])-1
        planes.append([maximum if (x+3*y+c+frame)%7==0 else 0 if (3*x+y+c+frame)%5==0 else ((x*173+y*271+c*193+frame*397)*maximum//1023)%(maximum+1) for y in range(height) for x in range(width)])
    return planes,map_planes(table,fmt,w,h,planes)



def map_planes(table, fmt, w, h, planes):
    sx,sy={1:(2,2),2:(2,1),3:(1,1)}[fmt]; cw,ch=w//sx,h//sy
    sizes=[(w,h),(cw,ch),(cw,ch)]
    output=[[],[],[]]
    for c,(width,height) in enumerate(sizes):
        for y in range(height):
            for x in range(width):
                if c==0:
                    cx,cy=x//sx,y//sy
                    # Tensor-product rational weights, with a single rounding
                    # after both axes. Repeated edge coordinates merge naturally.
                    wx=[(cx,F(1))] if sx==1 or x%2==0 else [(cx,F(1,2)),(min(cx+1,cw-1),F(1,2))]
                    other=max(0,cy-1) if y%2==0 else min(cy+1,ch-1)
                    wy=[(cy,F(1))] if sy==1 else [(cy,F(3,4)),(other,F(1,4))]
                    values=[planes[0][y*w+x]]+[rounded(sum(planes[k][py*cw+px]*a*b for px,a in wx for py,b in wy)) for k in (1,2)]
                else:
                    lx,ly=x*sx,y*sy
                    indices=[ly*w+lx] if sy==1 else [ly*w+lx,(ly+1)*w+lx]
                    values=[rounded(sum(F(planes[0][i],len(indices)) for i in indices)),planes[1][y*cw+x],planes[2][y*cw+x]]
                output[c].append(mapping(table,c,values))
    return output


def main():
    tables=json.loads((D/'hevc-layered-pps.json').read_text())['cases']
    chosen={'identity','bias','swap','clip','part-1-2-1','part-1-2-0','adapt','depth10','depth16','low-zero','mixed-output'}
    cases=[]
    for t in tables:
        label=t['prefix'].removeprefix('hevc-layered-pps-cm-')
        if label not in chosen: continue
        table=t['oracle']; depth=table['input'][0]
        assert depth==table['input'][1]
        for fmt in (1,2,3):
            for w,h in ((8,6),(2,2)):
                prefix=f'hevc-colour-map-{label}-{fmt}-{w}x{h}'
                samples=[picture(table,fmt,w,h,f) for f in (0,1)]
                colour={1:'420',2:'422',3:'444'}[fmt]+(f'p{depth}' if depth!=8 else '')
                raw=f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C{colour}\n'.encode()
                for planes,_ in samples:
                    flat=sum(planes,[])
                    raw+=b'FRAME\n'+(bytes(flat) if depth==8 else struct.pack('<'+'H'*len(flat),*flat))
                (D/(prefix+'.y4m')).write_bytes(raw)
                cases.append(dict(prefix=prefix,pps=t['prefix'],format=fmt,size=[w,h],depth=depth,frames=[dict(input=a,output=b) for a,b in samples]))
    (D/'hevc-colour-mapping-pictures.json').write_text(json.dumps(dict(cases=cases,provenance='Original two-frame Y4M patterns; exact rational tensor-product spatial oracle and saved owned LUT coefficients. No FFmpeg, external codec, network or private data. Colour mapping acceptance only, not multilayer decoding.'),indent=2)+'\n')
    print('Picture mapping cases:',len(cases))


if __name__=='__main__':main()
