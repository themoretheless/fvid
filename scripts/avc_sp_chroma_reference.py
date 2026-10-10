"""Scalar 4:2:0 SP fixture oracle; own matrix, bilinear prediction and filtering.

Only fixed fixture geometry: progressive SP macroblocks, QPC39, offsets0.
JM supplies unchanged luma cross-check bytes, never normative chroma pixels.
"""
from generate_avc_switching_chroma_fixtures import chroma


def filtered(plane, width, mode, slices):
    p=plane[:]
    if mode==1:return p
    # H.264 tables8-16/17 at QPC39: alpha71, beta12, tc0(bS3)=6.
    for my in range(width//8):
        for mx in range(width//8):
            for vertical in [True,False]:
                for edge in [0,4]:
                    external=edge==0
                    if external and ((mx==0 if vertical else my==0) or (mode==2 and slices==4)):continue
                    for line in range(8):
                        x=mx*8+(edge if vertical else line)
                        y=my*8+(line if vertical else edge)
                        at=y*width+x;step=1 if vertical else width
                        a=[p[at-(i+1)*step] for i in range(2)]
                        b=[p[at+i*step] for i in range(2)]
                        if abs(a[0]-b[0])>=71 or abs(a[1]-a[0])>=12 or abs(b[1]-b[0])>=12:continue
                        if external:
                            av=(2*a[1]+a[0]+b[1]+2)//4
                            bv=(2*b[1]+b[0]+a[1]+2)//4
                        else:
                            delta=max(-7,min(7,((b[0]-a[0])*4+a[1]-b[1]+4)//8))
                            av=max(0,min(255,a[0]+delta));bv=max(0,min(255,b[0]-delta))
                        p[at-step]=av;p[at]=bv
    return p


def prediction(plane, width, mx, my, mv):
    dx,dy=mv;out=[]
    def pixel(x,y):return plane[max(0,min(width-1,y))*width+max(0,min(width-1,x))]
    for y in range(8):
        for x in range(8):
            ix,fx=divmod((mx*8+x)*8+dx,8);iy,fy=divmod((my*8+y)*8+dy,8)
            out.append(((8-fx)*(8-fy)*pixel(ix,iy)+fx*(8-fy)*pixel(ix+1,iy)+(8-fx)*fy*pixel(ix,iy+1)+fx*fy*pixel(ix+1,iy+1)+32)//64)
    return out


def sequence(jm, planes, width, mode, slices=1, motion=False, sign_inside=True):
    chroma_width=width//2;luma_size=width*width;chroma_size=luma_size//4
    assert len(jm)==4*(luma_size+2*chroma_size)
    cb,cr=planes[1:];result=bytearray()
    for frame in range(4):
        if frame:
            qs=[0,26,39][frame-1]
            mv=[(1,-1),(3,2),(-5,7)][frame-1] if motion else (0,0)
            reconstructed=[]
            for plane in [cb,cr]:
                out=[0]*chroma_size
                for my in range(chroma_width//8):
                    for mx in range(chroma_width//8):
                        pred=prediction(plane,chroma_width,mx,my,mv)
                        samples=chroma(pred,[0]*4,[[0]*16 for _ in range(4)],39,qs,sign_inside)
                        for i,v in enumerate(samples):out[(my*8+i//8)*chroma_width+mx*8+i%8]=v
                reconstructed.append(filtered(out,chroma_width,mode,slices))
            cb,cr=reconstructed
        start=frame*(luma_size+2*chroma_size)
        result.extend(jm[start:start+luma_size]);result.extend(cb);result.extend(cr)
    return bytes(result)
