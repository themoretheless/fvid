"""Owned signed quarter-luma P prediction and explicit weights; scalar oracle."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-monochrome-fractional'
TAPS=[1,-5,20,20,-5,1]
def prediction(source,depth,mv,height=16):
    maximum=(1<<depth)-1
    sample=lambda x,y:source[max(0,min(height-1,y))*32+max(0,min(31,x))]
    clip=lambda n:max(0,min(maximum,n))
    def h(x,y):return clip((sum(t*sample(x+i-2,y)for i,t in enumerate(TAPS))+16)//32)
    def v(x,y):return clip((sum(t*sample(x,y+i-2)for i,t in enumerate(TAPS))+16)//32)
    def d(x,y):return clip((sum(a*b*sample(x+i-2,y+j-2)for i,a in enumerate(TAPS)for j,b in enumerate(TAPS))+512)//1024)
    out=[];fx,fy=mv[0]%4,mv[1]%4
    for row in range(height):
        for col in range(32):
            x,y=col+mv[0]//4,row+mv[1]//4
            if fx==fy==0:value=sample(x,y)
            elif fy==0:value=h(x,y)if fx==2 else (h(x,y)+sample(x+int(fx==3),y)+1)//2
            elif fx==0:value=v(x,y)if fy==2 else (v(x,y)+sample(x,y+int(fy==3))+1)//2
            elif fx==fy==2:value=d(x,y)
            elif fx==2:value=(d(x,y)+h(x,y+int(fy==3))+1)//2
            elif fy==2:value=(d(x,y)+v(x+int(fx==3),y)+1)//2
            else:value=(h(x,y+int(fy==3))+v(x+int(fx==3),y)+1)//2
            out.append(value)
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witnesses=0;weight_witnesses=0
    for depth in [8,10,14]:
        maximum=(1<<depth)-1
        source=[(x*1703+y*7907+x*y*113)&maximum for y in range(16)for x in range(32)]
        source[:4]=[0,maximum,0,maximum]
        for dx in range(-4,4):
            for dy in range(-4,4):
                predicted=prediction(source,depth,[dx,dy])
                nearest=[source[max(0,min(15,y+dy//4))*32+max(0,min(31,x+dx//4))]for y in range(16)for x in range(32)]
                changed=sum(a!=b for a,b in zip(predicted,nearest));witnesses+=changed
                if dx%4 or dy%4:assert changed>0
                for weighted in [False,True]:
                    name=f'mono-qpel-{depth}-{dx}-{dy}-w{int(weighted)}';config=configuration(0,depth,8,False,weighted=weighted)
                    frames=[]
                    for index in range(2):
                        b=Writer();b.ue(0);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
                        if index==0:b.ue(0)
                        b.u(index*2,4)
                        if index==0:b.u(0);b.u(0)
                        else:
                            b.u(0);b.u(0)
                            if weighted:b.ue(1);b.u(1);b.se(3);b.se(-5) # monochrome: no chroma denom/flags
                            b.u(0)
                        b.se(0);b.ue(1)
                        if index==0:
                            for mb in range(2):
                                b.ue(25);b.align()
                                for y in range(16):
                                    for x in range(16):b.u(source[y*32+mb*16+x],depth)
                        else:
                            for mb in range(2):
                                b.ue(0);b.ue(0);b.se(dx if mb==0 else 0);b.se(dy if mb==0 else 0);b.ue(0)
                        nal=b.nal(0x65 if index==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal
                        (OUT/f'{name}-{index}.packet').write_bytes(packet);frames.append((index,index==0,packet))
                    expected=[max(0,min(maximum,((3*v+1)//2)-(5<<(depth-8))))for v in predicted]if weighted else predicted
                    if weighted:
                        changes=sum(a!=b for a,b in zip(predicted,expected));assert changes>0;weight_witnesses+=changes
                    for suffix,data in [('avcc',config),('mp4',mux(config,frames,32,16,25)),('y16',struct.pack('<1024H',*(source+expected)))]: (OUT/(name+'.'+suffix)).write_bytes(data)
                    cases.append(dict(name=name,depth=depth,motion=[dx,dy],weighted=weighted,phase_witnesses=changed))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    (OUT/'coverage.json').write_text(json.dumps(dict(phase_witnesses=witnesses,weight_witnesses=weight_witnesses,cases=len(cases)),indent=2)+'\n')
if __name__=='__main__':generate()
