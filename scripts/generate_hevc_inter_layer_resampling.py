#!/usr/bin/env python3
"""Original Y4M reference videos and exact rational H.8.1.4.2 resampling oracle."""
import json,struct
from fractions import Fraction as F
from math import floor
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
# Normative H.265 v6 tables H.1/H.2, independent of production source.
L=[[0,0,0,64,0,0,0,0],[0,1,-3,63,4,-2,1,0],[-1,2,-5,62,8,-3,1,0],[-1,3,-8,60,13,-4,1,0],[-1,4,-10,58,17,-5,1,0],[-1,4,-11,52,26,-8,3,-1],[-1,3,-9,47,31,-10,4,-1],[-1,4,-11,45,34,-10,4,-1],[-1,4,-11,40,40,-11,4,-1],[-1,4,-10,34,45,-11,4,-1],[-1,4,-10,31,47,-9,3,-1],[-1,3,-8,26,52,-11,4,-1],[0,1,-5,17,58,-10,4,-1],[0,1,-4,13,60,-8,3,-1],[0,1,-3,8,62,-5,2,-1],[0,1,-2,4,63,-3,1,0]]
C=[[0,64,0,0],[-2,62,4,0],[-2,58,10,-2],[-4,56,14,-2],[-4,54,16,-2],[-6,52,20,-2],[-6,46,28,-4],[-4,42,30,-4],[-4,36,36,-4],[-4,30,42,-4],[-4,28,46,-6],[-2,20,52,-6],[-2,16,54,-4],[-2,14,56,-4],[-2,10,58,-2],[0,4,62,-2]]
SUB={0:(1,1),1:(2,2),2:(2,1),3:(1,1)}
def nearest(v):return floor(v+F(1,2))
def layout(size,fmt):return [size,[size[0]//SUB[fmt][0],size[1]//SUB[fmt][1]],[size[0]//SUB[fmt][0],size[1]//SUB[fmt][1]]]
def derive(c):
    cs,rs=SUB[c['current_format']],SUB[c['reference_format']]
    scaled=[c['scaled'][i]*cs[i%2] for i in range(4)]
    region=[c['region'][i]*rs[i%2] for i in range(4)]
    widths=[c['current_size'][a]-scaled[a]-scaled[a+2] for a in range(2)]
    refs=[c['reference_size'][a]-region[a]-region[a+2] for a in range(2)]
    phases=c['phases']
    if phases is None:
        vertical=8 if c['current_format']==3 else (4*widths[1]+refs[1]//2)//refs[1]+4
        phases=[0,0,8,vertical]
    phases=[phases[0],phases[1],phases[2]-8,phases[3]-8]
    coords=[]
    sizes=layout(c['current_size'],c['current_format'])
    for comp in range(2):
        axes=[]
        for a in range(2):
            csub,rsub=(cs[a],rs[a]) if comp else (1,1)
            rw,sw=refs[a]//rsub,widths[a]//csub
            scale=nearest(F(rw*65536,sw))
            add=-nearest(F(scale*phases[comp*2+a],16))
            co=scaled[a]//csub; ro=region[a]//rsub
            axes.append([nearest(F((p-co)*scale+add,4096))+ro*16 for p in range(sizes[comp][a])])
        coords.append(axes)
    return coords

def planes(c,f):
    arrays=[]
    for k,(w,h) in enumerate(layout(c['reference_size'],c['reference_format'])):
        maximum=(1 << c['reference_depth'][k!=0])-1
        arrays.append([maximum if (x+2*y+k+f)%5==0 else 0 if (3*x+y+k+f)%7==0 else ((x*197+y*311+k*157+f*389)*maximum//1023)%(maximum+1) for y in range(h) for x in range(w)])
    return arrays

def oracle(c,source,coords):
    # H.8.1.4.1 bypasses sample filtering for a completely equal reference.
    if c['reference_size']==c['current_size'] and c['reference_format']==c['current_format'] and c['reference_depth']==c['current_depth'] and c['scaled']==c['region'] and c['phases'] in (None,[0,0,8,8]):
        return [list(p) for p in source]
    result=[]
    for comp,(w,h) in enumerate(layout(c['current_size'],c['current_format'])):
        rw,rh=layout(c['reference_size'],c['reference_format'])[comp]
        taps,origin=(L,3) if comp==0 else (C,1)
        ax,ay=coords[comp!=0]
        # Independent rational horizontal matrix, quantized before the vertical
        # matrix. Collapsing these into one rounded 2D convolution is incorrect.
        horizontal=[]
        for sy in range(rh):
            row=[]
            for rx in ax:
                ix=rx//16; phase=rx%16
                total=sum(source[comp][sy*rw+max(0,min(rw-1,ix+i-origin))]*weight for i,weight in enumerate(taps[phase]))
                row.append(floor(F(total,1 << (c['reference_depth'][comp!=0]-8))))
            horizontal.append(row)
        output=[]
        for ry in ay:
            iy=ry//16; phase=ry%16
            for x in range(w):
                total=sum(horizontal[max(0,min(rh-1,iy+j-origin))][x]*weight for j,weight in enumerate(taps[phase]))
                v=nearest(F(total,1 << (20-c['current_depth'][comp!=0])))
                output.append(max(0,min((1 << c['current_depth'][comp!=0])-1,v)))
        result.append(output)
    return result

def motion_oracle(c,frame):
    cols=(c['reference_size'][0]+15)//16; rows=(c['reference_size'][1]+15)//16
    vectors=[-32768,-129,-128,-1,0,1,127,128,129,32767]
    source=[]
    for i in range(cols*rows):
        mode=(i+frame)%4
        source.append([None if mode in (0,2) else dict(reference=i%3,mv=[vectors[(i+frame)%10],vectors[(i*3+frame+2)%10]]),None if mode in (0,1) else dict(reference=(i+1)%4,mv=[vectors[(i*2+frame+4)%10],vectors[(i*5+frame+1)%10]])])
    cs,rs=SUB[c['current_format']],SUB[c['reference_format']]
    co=[c['scaled'][a]*cs[a] for a in range(2)];ro=[c['region'][a]*rs[a] for a in range(2)]
    sw=[c['current_size'][a]-c['scaled'][a]*cs[a]-c['scaled'][a+2]*cs[a] for a in range(2)]
    rw=[c['reference_size'][a]-c['region'][a]*rs[a]-c['region'][a+2]*rs[a] for a in range(2)]
    scales=[nearest(F(rw[a]*65536,sw[a])) for a in range(2)]
    output=[]; positions=[]
    for y in range((c['current_size'][1]+15)//16):
        for x in range((c['current_size'][0]+15)//16):
            point=[16*floor(F(nearest(F(((x,y)[a]*16+8-co[a])*scales[a],65536))+ro[a]+4,16)) for a in range(2)]
            positions.append(point)
            if any(point[a]<0 or point[a]>=c['reference_size'][a] for a in range(2)):output.append([None,None]);continue
            entry=source[point[1]//16*cols+point[0]//16];target=[]
            for vector in entry:
                if vector is None:target.append(None);continue
                values=[]
                for a,v in enumerate(vector['mv']):
                    if sw[a]!=rw[a]:
                        factor=min(4095,nearest(F(sw[a]*256,rw[a])))
                        product=factor*v
                        v=(1 if product>0 else -1 if product<0 else 0)*floor(F(abs(product)+127,256))
                        v=max(-32768,min(32767,v))
                    values.append(v)
                target.append(dict(reference=vector['reference'],mv=values))
            output.append(target)
    return source,output,positions

def main():
    cases=[]
    def add(label,**options):
        c=dict(prefix='hevc-inter-layer-resample-'+label,reference_size=[4,4],current_size=[64,64],reference_format=1,current_format=1,reference_depth=[8,8],current_depth=[8,8],scaled=[0,0,0,0],region=[0,0,0,0],phases=None,error=None)
        c.update(options);cases.append(c)
    for rf in (1,2,3):
        for cf in (1,2,3):
            for depth in (8,10,16):
                add(f'grid-{rf}-{cf}-{depth}',reference_format=rf,current_format=cf,reference_depth=[depth,depth],current_depth=[depth,depth],phases=[0,0,8,8])
    for fmt in (1,2,3):
        add(f'identity-{fmt}',reference_size=[8,6],current_size=[8,6],reference_format=fmt,current_format=fmt)
        add(f'depth-{fmt}',reference_size=[8,6],current_size=[8,6],reference_format=fmt,current_format=fmt,current_depth=[16,10])
        add(f'offset-{fmt}',reference_size=[8,6],current_size=[20,16],reference_format=fmt,current_format=fmt,scaled=[1,-1,2,1],region=[-1,1,1,-1],current_depth=[10,16])
        add(f'phase-{fmt}',reference_size=[8,6],current_size=[12,10],reference_format=fmt,current_format=fmt,phases=[31,17,63,0],reference_depth=[10,10],current_depth=[16,16])
    for fmt in (1,2,3):
        for depth in (10,16):
            add(f'identity-{fmt}-{depth}',reference_size=[8,6],current_size=[8,6],reference_format=fmt,current_format=fmt,reference_depth=[depth,depth],current_depth=[depth,depth])
    add('identity-offset16',reference_size=[8,6],current_size=[8,6],reference_depth=[16,16],current_depth=[16,16],scaled=[1,-1,-1,1],region=[1,-1,-1,1])
    add('motion-noninteger',reference_size=[64,48],current_size=[96,80],phases=[7,13,15,5])
    add('motion-outside',reference_size=[64,48],current_size=[128,96],scaled=[20,0,0,0],region=[-1,0,1,0])
    add('motion-partial',reference_size=[24,24],current_size=[40,40],reference_depth=[16,16],current_depth=[16,16])
    add('mixed-depth',reference_depth=[10,8],current_depth=[16,10],phases=[1,3,7,9])
    add('bad-inferred-phase',error='phase exceeds range')
    add('bad-downscale',current_size=[2,2],error='cannot downscale')
    add('bad-depth',reference_depth=[10,10],error='cannot reduce')
    add('bad-equal-phase',current_size=[4,4],phases=[1,0,8,8],error='require zero phase')
    add('bad-phase',phases=[32,0,8,8],error='phase exceeds range')
    add('bad-offset',scaled=[16384,0,0,0],error='offset exceeds range')
    add('bad-empty-region',region=[1,0,1,0],error='remove the picture region')
    add('bad-geometry',current_size=[63,64],error='geometry or depth')
    for c in cases:
        if c['error'] is None:c['coordinates']=derive(c)
        c['frames']=[]
        depth=max(c['reference_depth'])
        fmt={1:'420',2:'422',3:'444'}[c['reference_format']]+(f'p{depth}' if depth!=8 else '')
        w,h=c['reference_size']
        raw=f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C{fmt}\n'.encode()
        for f in (0,1):
            source=planes(c,f)
            output=oracle(c,source,c['coordinates']) if c['error'] is None else None
            motion_in,motion_out,motion_pos=motion_oracle(c,f) if c['error'] is None else (None,None,None)
            c['frames'].append(dict(input=source,output=output,motion_input=motion_in,motion_output=motion_out,motion_positions=motion_pos))
            flat=sum(source,[])
            raw+=b'FRAME\n'+(bytes(flat) if depth==8 else struct.pack('<'+'H'*len(flat),*flat))
        (D/(c['prefix']+'.y4m')).write_bytes(raw)
    (D/'hevc-inter-layer-resampling.json').write_text(json.dumps(dict(cases=cases,provenance='Original two-frame reference Y4M patterns and rational fixed-point spatial/filter oracle. Mixed-depth case uses a 10-bit Y4M storage envelope; reference depths are specified separately for the operation. No private data, external codec, FFmpeg or network. Resampling acceptance, not scalable decoding.'),separators=(',',':'))+'\n')
    print('Resampling cases:',len(cases),flush=True)
if __name__=='__main__':main()
