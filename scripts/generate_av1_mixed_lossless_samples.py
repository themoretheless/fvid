#!/usr/bin/env python3
"""Owned 16x16 blocks mixing lossless/lossy AV1 segments, no private inputs."""
import argparse,hashlib,json,re,subprocess,functools
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,show,webm
from generate_av1_segmentation_map_samples import CDF,unmap,adapt_symbols
ROOT=Path(__file__).resolve().parents[1]
TABLES=(ROOT/'src/codec/av1_cdfs.rs').read_text()
@functools.lru_cache(maxsize=None)
def table(name,offset,length):
    raw=re.search(r'const '+name+r': &\[u16\] = &\[(.*?)\];',TABLES,re.S).group(1)
    values=list(map(int,re.findall(r'\d+',raw)));row=values[offset:offset+length];assert len(row)==length and row[-2]==32768 and row[-1]==0
    return row[:-1]
def encode(writer,base,mask,selected,adaptive,residual=0,inter=False,reference=1,motion=0,forced_reference=False):
    vector=(0,motion) if isinstance(motion,int) else tuple(motion)
    assert len(vector)==2 and all(v in [-8,-4,-2,0,2,4,8] for v in vector)
    symbols=[];grid=[0]*64;txs={};above=[[(0,0)]*8 for _ in range(3)];left=[[(0,0)]*8 for _ in range(3)];qi=0 if base<=20 else 1 if base<=60 else 2 if base<=120 else 3
    def s(id,indices,name,offset,length,value):symbols.append(dict(model=[id,indices],cdf=table(name,offset,length),symbol=value))
    s(8,[0],'DEFAULT_PARTITION_W32_CDF',0,11,3)
    for i,(x,y) in enumerate([(0,0),(4,0),(0,4),(4,4)]):
        segment=(mask>>i)&1;lossless=segment==0
        s(7,[0],'DEFAULT_PARTITION_W16_CDF',0,11,0)
        if not forced_reference:s(32,[0],'DEFAULT_SKIP_CDF',0,3,0)
        u=grid[(y-1)*8+x] if y else None;l=grid[y*8+x-1] if x else None;ul=grid[(y-1)*8+x-1] if x and y else None
        pred=(l if u is None else u if l is None or ul==u else l) or 0
        ctx=0 if ul is None else 2 if ul==u==l else 1 if ul==u or ul==l or u==l else 0
        diff=next(d for d in range(2) if unmap(d,pred,2)==segment)
        symbols.append(dict(model=[18,[ctx]],cdf=CDF[ctx],symbol=diff))
        if forced_reference:s(32,[0],'DEFAULT_SKIP_CDF',0,3,0)
        if inter and not (forced_reference and reference==0):
            if not forced_reference:s(29,[0],'DEFAULT_IS_INTER_CDF',0,3,1)
            n=int(x>0)+int(y>0)
            def bit(branch,a,b,value):
                ac=n*int(reference in a);bc=n*int(reference in b);ctx=0 if ac<bc else 2 if ac>bc else 1
                s(35,[ctx,branch],'DEFAULT_SINGLE_REF_CDF',(ctx*6+branch)*3,3,value)
            if not forced_reference:
                backward=reference>=5;bit(0,[1,2,3,4],[5,6,7],int(backward))
                if backward:
                    bit(1,[5,6],[7],int(reference==7))
                    if reference!=7:bit(5,[5],[6],int(reference==6))
                else:
                    high=reference>=3;bit(2,[1,2],[3,4],int(high));bit(4 if high else 3,[3] if high else [1],[4] if high else [2],int(reference in [2,4]))
            newctx=([0,2,2,5] if any(vector) else [0,3,3,5])[i]
            moving=bool(any(vector) and i==0)
            s(25,[newctx],'DEFAULT_NEW_MV_CDF',newctx*3,3,0 if moving else 1)
            if moving:
                joint=3 if all(vector) else 2 if vector[0] else 1
                s(39,[0],'DEFAULT_MV_JOINT_CDF',0,5,joint)
                for comp,magnitude in enumerate(vector):
                    if not magnitude:continue
                    s(22,[0,comp],'DEFAULT_MV_SIGN_CDF',0,3,int(magnitude<0))
                    s(40,[0,comp],'DEFAULT_MV_CLASS_CDF',comp*12,12,0)
                    s(24,[0,comp],'DEFAULT_MV_CLASS0_BIT_CDF',0,3,0)
                    s(41,[0,comp,0],'DEFAULT_MV_CLASS0_FR_CDF',comp*10,5,abs(magnitude)//2-1)
            else:s(26,[0],'DEFAULT_ZERO_MV_CDF',0,3,0)
            if selected and not lossless:
                ctx=12+int(y>0 and txs[(x,y-4)]<16)+int(x>0 and txs[(x-4,y)]<16)
                s(15,[ctx],'DEFAULT_TXFM_SPLIT_CDF',ctx*3,3,0)
        else:
            if inter:s(1,[2],'DEFAULT_Y_MODE_CDF',28,14,0)
            else:s(0,[0,0],'DEFAULT_INTRA_FRAME_Y_MODE_CDF',0,14,0)
            s(2 if lossless else 3,[0],'DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF' if lossless else 'DEFAULT_UV_MODE_CFL_ALLOWED_CDF',0,14 if lossless else 15,0)
            if selected and not lossless:
                ctx=int(y>0 and txs[(x,y-4)]>=16)+int(x>0 and txs[(x-4,y)]>=16)
                s(12,[ctx],'DEFAULT_TX_16X16_CDF',ctx*4,4,0)
        txs[(x,y)]=4 if lossless else 16
        for p in range(3):
            bw=4 if p==0 else 2;bx=x if p==0 else x//2;by=y if p==0 else y//2
            step=1 if lossless else bw;txctx=0 if lossless else 2 if p==0 else 1
            for yy in range(by,by+bw,step):
                for xx in range(bx,bx+bw,step):
                    top=above[p][xx:xx+step];side=left[p][yy:yy+step];t=max(v[0] for v in top);l=max(v[0] for v in side)
                    if p==0:
                        ctx=0 if bw==step else 1 if t==0 and l==0 else 2+int(max(t,l)>3) if t==0 or l==0 else 4 if max(t,l)<=3 else 5 if min(t,l)<=3 else 6
                    else:ctx=7+int(any(a|b for a,b in top))+int(any(a|b for a,b in side))+3*int(bw>step)
                    nonzero=bool(residual) and lossless and xx==bx+bw-1 and yy==by+bw-1
                    s(83,[txctx,ctx],'DEFAULT_TXB_SKIP_CDF',qi*195+(txctx*13+ctx)*3,3,int(not nonzero))
                    state=(0,0)
                    if nonzero:
                        pt=int(p>0);dc=sum(-1 if sign==1 else 1 if sign==2 else 0 for _,sign in top+side);dcctx=1 if dc<0 else 2 if dc>0 else 0
                        s(84,[pt,0],'DEFAULT_EOB_PT_16_CDF',qi*24+pt*12,6,0)
                        level=abs(residual);assert 1<=level<=14
                        s(93,[0,pt,0],'DEFAULT_COEFF_BASE_EOB_CDF',qi*160+pt*16,4,min(level,3)-1)
                        if level>=3:
                            rest=level-3
                            for _ in range(4):
                                v=min(rest,3);s(95,[0,pt,0],'DEFAULT_COEFF_BR_CDF',qi*1050+pt*105,5,v);rest-=v
                                if v<3:break
                        s(92,[pt,dcctx],'DEFAULT_DC_SIGN_CDF',qi*18+(pt*3+dcctx)*3,3,int(residual<0))
                        state=(level,1 if residual<0 else 2)
                    for k in range(step):above[p][xx+k]=state;left[p][yy+k]=state
        for yy in range(y,y+4):
            for xx in range(x,x+4):grid[yy*8+xx]=segment
    if adaptive:symbols,_=adapt_symbols(symbols)
    text=''.join(f"{len(r['cdf'])} {r['symbol']} "+' '.join(map(str,r['cdf']))+'\n' for r in symbols)
    return subprocess.run([str(writer)],input=text.encode(),stdout=subprocess.PIPE,check=True).stdout,grid

def key(entropy,base,selected,adaptive,kind=0,refresh=255):
    b=Bits();b.u(0);b.u(kind,2);b.u(0);b.u(1);b.u(1);b.u(int(not adaptive));b.u(0);b.u(refresh,8);b.u(0)
    if adaptive:b.u(1)
    b.u(1);b.u(base,8);b.u(0,4);b.u(1)
    for seg in range(8):
        for feature in range(8):
            active=seg<2 and feature==0;b.u(int(active))
            if active:b.u(-base if seg==0 else 0,9)
    b.u(0);b.u(0,16);b.u(int(selected));b.u(0)
    return obu(6,b.bytes()+entropy)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=ROOT/'tests/fixtures/playback-errors';records=[]
    for residual in [0,1,-1]:
        for base in [1,64,255]:
            for mask in range(1,15):
                for selected in [False,True]:
                    for adaptive in [False,True]:
                        entropy,grid=encode(a.writer,base,mask,selected,adaptive,residual)
                        data=sequence(False,False,False)+key(entropy,base,selected,adaptive)+show(False,False,False,0,0)
                        name=f'av1-mixed-lossless-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}'+(f'-dc{residual}' if residual else '');file=name+'.obu';reference=name+'.yuv';wrapped=name+'.webm'
                        (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                        subprocess.run([str(a.oracle),str(root/file),'1',str(root/reference)],check=True)
                        pixels=(root/reference).read_bytes();assert len(pixels)==1536
                        assert (pixels==bytes([128])*1536)==(residual==0)
                        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=reference,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,map=grid))
    (root/'av1-mixed-lossless-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
