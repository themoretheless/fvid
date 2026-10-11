#!/usr/bin/env python3
"""Original VP9 AC videos and independent 4x4 matrix pixel oracles, offline."""
import json
from functools import lru_cache
from generate_vp9_chroma import DEST, frame, table as load_table, webm, coloured
from generate_vp9_segmentation import find, PROBS
from generate_vp9_compound import profile_bits, literal, neutral_intra, webm as inter_webm
class BooleanWriter:
    """Exact lower-endpoint Boolean coder with stable-prefix byte flushing."""
    def __init__(self):
        self.low=0;self.range=255;self.width=8;self.output=bytearray()
        self.write(128,0)
    def write(self,p,bit):
        split=1+((self.range-1)*p>>8)
        if bit:self.low+=split;self.range-=split
        else:self.range=split
        shift=8-self.range.bit_length()
        self.range<<=shift;self.low<<=shift;self.width+=shift
        while self.width>=40:
            remain=self.width-16
            prefix=self.low>>remain
            if prefix!=(self.low+self.range-1)>>remain:break
            self.output+=prefix.to_bytes(2,'big')
            self.low&=(1<<remain)-1;self.width=remain
    def finish(self):
        bits=f'{self.low:0{self.width}b}';bits+='0'*(-len(bits)%8)
        return bytes(self.output)+bytes(int(bits[i:i+8],2) for i in range(0,len(bits),8))+bytes(8)

table=lru_cache(maxsize=None)(load_table)
# VP9 v0.7 sections 6.4.24, 10.5: normative scan/bands and coefficient tree.
SCAN=[0,4,1,5,8,2,12,9,3,6,13,10,7,14,11,15]
BAND=[0,1,1,2,2,2,3,3,3,3,4,4,4,5,5,5]
TREE=[0,2,-1,4,6,10,-2,8,-3,-4,12,14,-5,-6,16,18,-7,-8,-9,-10]
BASE=[0,1,2,3,4,5,7,11,19,35,67]
EXTRA=[[],[159],[165,145],[173,148,140],[176,155,140,135],[180,157,141,134,130],[254,254,254,252,249,243,230,196,177,153,140,133,130,129]]
ENERGY=[0,1,2,3,3,4,4,5,5,5,5]
COVERAGE={"positions":set(),"tokens":set(),"signs":set(),"category6_high_bit_depths":set()}
H=[[1,1,1,1],[1,1,-1,-1],[1,-1,-1,1],[1,-1,1,-1]]

@lru_cache(maxsize=4096)
def wht_matrix(coefficients,dc,ac):
    # Dense matrix oracle, deliberately independent of the production butterfly.
    scaled=[(v*(dc if i==0 else ac))//4 for i,v in enumerate(coefficients)]
    rows=[[ (sum(H[j][k]*scaled[y*4+k] for k in range(4))+int(j==0))//2 for j in range(4)] for y in range(4)]
    return [[(sum(H[y][k]*rows[k][x] for k in range(4))+int(y==0))//2 for x in range(4)] for y in range(4)]

@lru_cache(maxsize=4096)
def dct_matrix(coefficients,dc,ac):
    # Independent 4-point even/odd matrix products. Round the products before
    # summing output rows; collapsing those roundings into a floating DCT is wrong.
    even=[[11585,0,11585,0],[11585,0,-11585,0]]
    odd=[[0,15137,0,6270],[0,6270,0,-15137]]
    def line(v):
        e=[(sum(a*b for a,b in zip(row,v))+8192)//16384 for row in even]
        o=[(sum(a*b for a,b in zip(row,v))+8192)//16384 for row in odd]
        return [e[0]+o[0],e[1]+o[1],e[1]-o[1],e[0]-o[0]]
    scaled=[v*(dc if i==0 else ac) for i,v in enumerate(coefficients)]
    rows=[line(scaled[y*4:y*4+4]) for y in range(4)]
    columns=[line([rows[y][x] for y in range(4)]) for x in range(4)]
    return [[(columns[x][y]+8)//16 for x in range(4)] for y in range(4)]

def token(value):
    v=abs(value)
    if v<=4:return v
    return max(i for i in range(5,11) if BASE[i]<=v)

def coefficients(b,values,initial,depth,chroma,probs,pareto,inter=False):
    cache=[0]*16;check=True;last=max((i for i,p in enumerate(SCAN) if values[p]),default=-1)
    for c,pos in enumerate(SCAN):
        y,x=divmod(pos,4)
        if c==0:ctx=initial
        else:
            a=pos-4 if y else pos-1;d=pos-1 if x else pos-4
            ctx=(1+cache[a]+cache[d])//2
        p=probs[chroma][int(inter)][BAND[c]][ctx]
        if check:
            b.write(p[0],int(c<=last))
            if c>last:break
        t=token(values[pos]);path=find(TREE,t)
        for node,bit in path:
            n=node//2
            if n<2:prob=p[1+n]
            else:
                q=p[2];i=(q-1)//2
                prob=pareto[i][n-2] if q&1 else (pareto[i][n-2]+pareto[i+1][n-2])//2
            b.write(prob,bit)
        cache[pos]=ENERGY[t];check=t!=0
        if t:
            v=abs(values[pos])-BASE[t]
            if t==10:
                for e in range(depth-8):b.write(255,(v>>(5+depth-e))&1)
            if t>=5:
                extra=EXTRA[t-4]
                for e,p in enumerate(extra):b.write(p,(v>>(len(extra)-1-e))&1)
            b.write(128,int(values[pos]<0))

def pattern(index,plane,mode,depth,q):
    mags=[1,2,3,4,5,6,8,12,20,36,68,324]
    if q==0 and depth>8:mags.append(16452 if depth==10 else 65604)
    values=[0]*16
    if mode in [0,1]:
        p=(index+plane*5)%16
        values[p]=mags[(index//16+plane)%len(mags)]*(-1 if mode else 1)
    elif mode==2:
        for k,p in enumerate([0,(index%15)+1,15]):
            values[p]+=mags[(index+k+plane)%len(mags)]*(-1 if (index+k)&1 else 1)
    elif index%7:
        # Dense, alternating signed coefficients and internal zero runs.
        for p in range(16):
            if (index+p)%3:values[p]=mags[(index+p+plane)%len(mags)]*(-1 if p&1 else 1)
    return values

def picture(depth,chroma,mode,q,inter=False,base=0,reference=None):
    sub={'420':(1,1),'422':(1,0),'444':(0,0),'440':(0,1)}[chroma]
    b=BooleanWriter();b.write(table('DEFAULT_PARTITION_PROBS' if inter else 'KF_PARTITION_PROBS')[12][0],0)
    if q:
        for p in [91,137,73]:b.write(p,0)
    b.write(table('DEFAULT_SKIP_PROB')[0],0)
    if inter:
        b.write(table('DEFAULT_IS_INTER_PROB')[0],1)
        if reference is None:b.write(table('DEFAULT_SINGLE_REF_PROB')[2][0],0)
        else:b.write(table('DEFAULT_COMP_REF_PROB')[2],1)
        b.write(table('DEFAULT_INTER_MODE_PROBS')[2][0],0)
    else:
        b.write(table('KF_Y_MODE_PROBS')[0][0][0],0);b.write(table('KF_UV_MODE_PROBS')[0][0],0)
    probs=table('DEFAULT_COEF_PROBS')[0];pareto=table('PARETO_TABLE')
    index=base+q
    dc=table('DC_QLOOKUP')[(depth-8)//2][index];ac=table('AC_QLOOKUP')[(depth-8)//2][index]
    oracle=bytearray();offset=0
    for plane in range(3):
        sx,sy=sub if plane else (0,0);w,h=64>>sx,64>>sy
        pixels=[0]*(w*h);above=[False]*(w//4);left=[False]*(h//4)
        for y in range(0,h,4):
            for x in range(0,w,4):
                vals=pattern(y//4*(w//4)+x//4,plane,mode,depth,index)
                COVERAGE['positions'].update(i for i,v in enumerate(vals) if v)
                COVERAGE['tokens'].update(token(v) for v in vals)
                COVERAGE['signs'].update(1 if v>0 else -1 for v in vals if v)
                if any(abs(v)>=67+(1<<14) for v in vals):COVERAGE['category6_high_bit_depths'].add(depth)
                coefficients(b,vals,int(above[x//4])+int(left[y//4]),depth,int(plane>0),probs,pareto,inter)
                above[x//4]=left[y//4]=any(vals)
                neighbours=[]
                if y:neighbours+=pixels[(y-1)*w+x:(y-1)*w+x+4]
                if x:neighbours += [pixels[(y+j)*w+x-1] for j in range(4)]
                pred=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 1<<(depth-1)
                if inter:pred=1<<(depth-1)
                residual=(dct_matrix if base else wht_matrix)(tuple(vals),dc,ac)
                for yy in range(4):
                    for xx in range(4):
                        prediction=pred
                        if reference is not None:
                            at=offset+((y+yy)*w+x+xx)*2
                            prediction=(int.from_bytes(reference[at:at+2],'little')+(1<<(depth-1))+1)//2
                        pixels[(y+yy)*w+x+xx]=max(0,min((1<<depth)-1,prediction+residual[yy][xx]))
        oracle+=b''.join(v.to_bytes(2,'little') for v in pixels)
        offset+=w*h*2
    seg='0'
    if q:seg='11'+''.join('1'+f'{p:08b}' for p in PROBS)+'0'+'10'+'1'+f'{q:08b}'+'0'+'000'+'0000'*7
    if inter:
        bits,_=profile_bits(depth,chroma)
        bits+='0110'+'00'+'00000000'+('0000'*3 if reference is None else '0000'+'0010'+'0001')+'1'+'0'+'0'+'011'
        compressed=bytes(32)
        if reference is not None:
            c=BooleanWriter()
            if base:c.write(128,0);c.write(128,0)
            c.write(128,0)
            for _ in range(3+21+4):c.write(252,0)
            c.write(128,1);c.write(128,0)
            for _ in range(5+36+48+3+44+18):c.write(252,0)
            compressed=c.finish()
        bits+='0100'+'0'*10+f'{base:08b}'+'000'+seg+'0'+f'{len(compressed):016b}'
        packet=literal(bits)+compressed+b.finish()
    else:packet=frame(depth,chroma,tile=b.finish(),segmentation=seg,base_q=base)
    return packet,oracle

def main():
    cases=[]
    for depth in [8,10,12]:
        for chroma in ['420','422','444','440']:
            seed,reference=coloured(depth,chroma)
            for base in [0,20]:
                for q in [0,6]:
                    frames=[];entries=[]
                    inter_frames=[frame(depth,chroma)];inter_entries=[]
                    compound_frames=[seed,neutral_intra(depth,chroma)];compound_entries=[]
                    for mode in range(4):
                        packet,expected=picture(depth,chroma,mode,q,base=base)
                        stem=f'vp9-ac-{chroma}-{depth}-{q}-{mode}'+('-dct' if base else '')
                        (DEST/f'{stem}.vp9').write_bytes(packet);(DEST/f'{stem}.yuv16').write_bytes(expected)
                        inter_packet,inter_expected=picture(depth,chroma,mode,q,inter=True,base=base)
                        (DEST/f'{stem}-inter.vp9').write_bytes(inter_packet);(DEST/f'{stem}-inter.yuv16').write_bytes(inter_expected)
                        inter_frames.append(inter_packet);inter_entries.append(dict(file=f'{stem}-inter.vp9',oracle=f'{stem}-inter.yuv16',mode=mode))
                        compound_packet,compound_expected=picture(depth,chroma,mode,q,inter=True,base=base,reference=reference)
                        (DEST/f'{stem}-compound.vp9').write_bytes(compound_packet);(DEST/f'{stem}-compound.yuv16').write_bytes(compound_expected)
                        compound_frames.append(compound_packet);compound_entries.append(dict(file=f'{stem}-compound.vp9',oracle=f'{stem}-compound.yuv16',mode=mode))
                        frames.append(packet);entries.append(dict(file=f'{stem}.vp9',oracle=f'{stem}.yuv16',mode=mode))
                    name=f'vp9-ac-{chroma}-{depth}-{q}'+('-dct' if base else '')+'.webm';(DEST/name).write_bytes(webm(frames))
                    inter_name=f'vp9-ac-{chroma}-{depth}-{q}-inter'+('-dct' if base else '')+'.webm';(DEST/inter_name).write_bytes(inter_webm(inter_frames))
                    compound_name=f'vp9-ac-{chroma}-{depth}-{q}-compound'+('-dct' if base else '')+'.webm';(DEST/compound_name).write_bytes(inter_webm(compound_frames))
                    cases.append(dict(compound_file=compound_name,compound_frames=compound_entries,file=name,depth=depth,chroma=chroma,q=q,base=base,frames=entries,inter_file=inter_name,inter_frames=inter_entries))
    assert COVERAGE['positions']==set(range(16)) and COVERAGE['tokens']==set(range(11))
    assert COVERAGE['category6_high_bit_depths']=={10,12}
    (DEST/'vp9-ac.json').write_text(json.dumps(dict(cases=cases,coverage={k:sorted(v) for k,v in COVERAGE.items()},provenance='Original coefficient tokens, normative scan/bands, owned Boolean writer; independent dense Hadamard and even/odd DCT matrix oracles with exact signed rounding, dequantization, DC prediction and clipping. No private media, codec process, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
