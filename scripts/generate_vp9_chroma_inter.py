#!/usr/bin/env python3
"""Original VP9 single-reference motion pictures and independent bilinear oracles."""
import json
from generate_vp9_chroma import BooleanWriter, DEST, coloured, table
from generate_audio_resample_window_fixture import ebml
SUBS={'420':(1,1),'422':(1,0),'444':(0,0),'440':(0,1)}
def literal(bits):
    bits += '0'*(-len(bits)%8)
    return bytes(int(bits[i:i+8],2) for i in range(0,len(bits),8))
def vector(b,mv):
    # The authored nonzero vectors have both components, precision 1/4 pel.
    for p in table('DEFAULT_MV_JOINT_PROBS'): b.write(p,1)
    for comp, value in enumerate(mv):
        b.write(table('DEFAULT_MV_SIGN_PROB')[comp],int(value<0))
        mag=abs(value); d=mag-1
        cls=int(d>=16)
        probs=table('DEFAULT_MV_CLASS_PROBS')[comp]
        b.write(probs[0],cls)
        if cls: b.write(probs[1],0)
        if cls:
            d-=16
            b.write(table('DEFAULT_MV_BITS_PROB')[comp][0],d>>3)
            frac=(d>>1)&3
            ps=table('DEFAULT_MV_FR_PROBS')[comp]
        else:
            digit=d>>3
            b.write(table('DEFAULT_MV_CLASS0_BIT_PROB')[comp],digit)
            frac=(d>>1)&3
            ps=table('DEFAULT_MV_CLASS0_FR_PROBS')[comp][digit]
        path=['0','10','110','111'][frac]
        for p,bit in zip(ps,path): b.write(p,int(bit))
        # high_precision_mv=0 implies the low magnitude bit, equal to one.
        assert mag in [6,10,16] and d&1

def inter(depth,chroma,mv,sub8=False,raw_filter=3):
    profile=(0 if chroma=='420' else 1)+(2 if depth>8 else 0)
    bits='10'+str(profile&1)+str(profile>>1)
    if profile==3: bits+='0'
    bits+='0110'+'00'+'00000000'+'0000'*3
    bits+=('000'+f'{7:016b}{7:016b}' if sub8 else '1')+'0'+'0'+'0'+f'{raw_filter:02b}' # reference/render size, low-precision, fixed filter
    bits+='0100' # context controls: no refresh, parallel, slot zero
    bits+='0'*10+'0'*8+'000'+'0'+'0'+f'{32:016b}'
    b=BooleanWriter()
    if sub8:
        for p in table('DEFAULT_PARTITION_PROBS')[0]: b.write(p,1)
    else: b.write(table('DEFAULT_PARTITION_PROBS')[12][0],0)
    b.write(table('DEFAULT_SKIP_PROB')[0],1)
    b.write(table('DEFAULT_IS_INTER_PROB')[0],1)
    b.write(table('DEFAULT_SINGLE_REF_PROB')[2][0],0)
    mode=table('DEFAULT_INTER_MODE_PROBS')[2]
    for motion in mv if sub8 else [mv]:
        if motion==[0,0]: b.write(mode[0],0) # ZEROMV
        else:
            for p in mode: b.write(p,1) # NEWMV with zero predictor
            vector(b,motion)
    return literal(bits)+bytes(32)+b.finish()
def oracle(data,depth,chroma,mv):
    values=[int.from_bytes(data[i:i+2],'little') for i in range(0,len(data),2)]
    result=bytearray();at=0
    for plane in range(3):
        sx,sy=SUBS[chroma] if plane else (0,0)
        w,h=64>>sx,64>>sy; samples=values[at:at+w*h];at+=w*h
        dy=(2*mv[0])>>sy;dx=(2*mv[1])>>sx
        def sample(x,y): return samples[max(0,min(h-1,y))*w+max(0,min(w-1,x))]
        for y in range(h):
            iy,fy=divmod(y*16+dy,16)
            for x in range(w):
                ix,fx=divmod(x*16+dx,16)
                top=(sample(ix,iy)*(16-fx)+sample(ix+1,iy)*fx+8)//16
                bottom=(sample(ix,iy+1)*(16-fx)+sample(ix+1,iy+1)*fx+8)//16
                value=(top*(16-fy)+bottom*fy+8)//16
                result+=value.to_bytes(2,'little')
    assert at==len(values)
    return result
def filtered_oracle(data,depth,chroma,mv,raw_filter):
    values=[int.from_bytes(data[i:i+2],'little') for i in range(0,len(data),2)]
    taps=table('SUBPEL_FILTERS')[[1,0,2,3][raw_filter]]
    result=bytearray();at=0; maximum=(1<<depth)-1
    for plane in range(3):
        sx,sy=SUBS[chroma] if plane else (0,0)
        w,h=64>>sx,64>>sy;samples=values[at:at+w*h];at+=w*h
        dx=(2*mv[1])>>sx;dy=(2*mv[0])>>sy
        def sample(x,y): return samples[max(0,min(h-1,y))*w+max(0,min(w-1,x))]
        def clipped(value): return max(0,min(maximum,(value+64)//128))
        for y in range(h):
            iy,fy=divmod(y*16+dy,16)
            for x in range(w):
                ix,fx=divmod(x*16+dx,16)
                horizontal=[clipped(sum(sample(ix+k-3,iy+j-3)*taps[fx][k] for k in range(8))) for j in range(8)]
                result+=clipped(sum(horizontal[j]*taps[fy][j] for j in range(8))).to_bytes(2,'little')
    assert at==len(values)
    return result
def sub8_oracle(data,chroma,mvs):
    values=[int.from_bytes(data[i:i+2],'little') for i in range(0,len(data),2)]
    result=bytearray();at=0
    for plane in range(3):
        sx,sy=SUBS[chroma] if plane else (0,0)
        rw,rh=16>>sx,16>>sy;w,h=8>>sx,8>>sy
        samples=values[at:at+rw*rh];at+=rw*rh
        def sample(x,y): return samples[max(0,min(rh-1,y))*rw+max(0,min(rw-1,x))]
        for y in range(h):
            for x in range(w):
                idx=(y//4)*(2>>sx)+x//4
                ids=([idx] if (sx,sy)==(0,0) else [idx,idx+2] if (sx,sy)==(0,1)
                     else [idx,idx+1] if (sx,sy)==(1,0) else [0,1,2,3])
                sums=[sum(mvs[i][c] for i in ids) for c in range(2)]
                mv=[(1 if s>=0 else -1)*((abs(s)+len(ids)//2)//len(ids)) for s in sums]
                # Reference dimensions are exactly twice the coded dimensions.
                ix,fx=divmod(32*x+2*((2*mv[1])>>sx),16)
                iy,fy=divmod(32*y+2*((2*mv[0])>>sy),16)
                top=(sample(ix,iy)*(16-fx)+sample(ix+1,iy)*fx+8)//16
                bottom=(sample(ix,iy+1)*(16-fx)+sample(ix+1,iy+1)*fx+8)//16
                result+=((top*(16-fy)+bottom*fy+8)//16).to_bytes(2,'little')
    assert at==len(values)
    return result
def webm(packets,side=64):
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(333333333).to_bytes(4,'big'))
    track+=ebml('e0',ebml('b0',bytes([side]))+ebml('ba',bytes([side])))
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets): cluster+=ebml('a3',b'\x81'+(333*i).to_bytes(2,'big')+bytes([128 if i==0 else 0])+p)
    info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
def main():
    cases=[]
    for depth in [8,10,12]:
        for chroma in SUBS:
            key,reference=coloured(depth,chroma)
            for name,mv in [('zero',[0,0]),('integer',[16,-16]),('fractional',[6,-10]),('opposite',[-6,10])]:
                stem=f'vp9-chroma-inter-{chroma}-{depth}-{name}'
                (DEST/f'{stem}.webm').write_bytes(webm([key,inter(depth,chroma,mv)]))
                (DEST/f'{stem}.yuv16').write_bytes(oracle(reference,depth,chroma,mv))
                cases.append(dict(file=f'{stem}.webm',oracle=f'{stem}.yuv16',depth=depth,chroma=chroma,motion=mv))
            for raw_filter,label in [(0,'smooth'),(1,'regular'),(2,'sharp')]:
                mv=[6,-10];stem=f'vp9-chroma-inter-{chroma}-{depth}-filter-{label}'
                (DEST/f'{stem}.webm').write_bytes(webm([key,inter(depth,chroma,mv,raw_filter=raw_filter)]))
                (DEST/f'{stem}.yuv16').write_bytes(filtered_oracle(reference,depth,chroma,mv,raw_filter))
                cases.append(dict(file=f'{stem}.webm',oracle=f'{stem}.yuv16',depth=depth,chroma=chroma,motion=mv,raw_filter=raw_filter))
            mvs=[[6,-10],[-6,10],[16,-16],[-16,16]]
            small,reference=coloured(depth,chroma,side=16)
            stem=f'vp9-chroma-inter-{chroma}-{depth}-sub8'
            (DEST/f'{stem}.webm').write_bytes(webm([small,inter(depth,chroma,mvs,sub8=True)],side=16))
            (DEST/f'{stem}.yuv16').write_bytes(sub8_oracle(reference,chroma,mvs))
            cases.append(dict(file=f'{stem}.webm',oracle=f'{stem}.yuv16',depth=depth,chroma=chroma,motion=mvs,size=[8,8],key_size=[16,16]))
    (DEST/'vp9-chroma-inter.json').write_text(json.dumps(dict(cases=cases,provenance='Original coloured lossless keyframe and owned Boolean-coded single-reference skipped inter picture. Independent Python bilinear interpolation with signed coordinates, two-pass rounding and replicated borders supplies every expected sample. No private media, external codec, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
