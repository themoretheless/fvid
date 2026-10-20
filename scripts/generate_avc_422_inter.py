"""Owned 422 P reference geometry and signed fractional motion witnesses."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_monochrome_fractional import prediction
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-inter'
def chroma(source,depth,mv):
    def at(x,y):return source[max(0,min(15,y))*16+max(0,min(15,x))]
    out=[]
    for y in range(16):
        for x in range(16):
            u,v=x*8+mv[0],y*8+2*mv[1];ix,iy=u//8,v//8;fx,fy=u%8,v%8
            out.append(((8-fx)*(8-fy)*at(ix,iy)+fx*(8-fy)*at(ix+1,iy)+(8-fx)*fy*at(ix,iy+1)+fx*fy*at(ix+1,iy+1)+32)//64)
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for cabac,mv,weighted in [(False,v,w)for v in [(0,0),(-4,0),(4,0),(0,-4),(0,4),(1,1),(-3,-1),(3,2)]for w in [False,True]]+[(True,(0,0),False)]:
            name=f'422-p-y{yd}-c{cd}-e{int(cabac)}-x{mv[0]}-y{mv[1]}'+('-weighted'if weighted else'');config=configuration(2,yd,cd,False,cabac,weighted)
            planes=[[(x*1703+y*7907+x*y*113+p*43)&((1<<depth)-1)for y in range(16)for x in range(w)]for p,(depth,w)in enumerate([(yd,32),(cd,16),(cd,16)])]
            frames=[]
            for index in range(2):
                nals=[]
                for address in (range(2)if cabac and index==0 else[0]):
                    b=Writer();b.ue(address);b.ue(2 if index==0 else 0);b.ue(0);b.u(index,4)
                    if index==0:b.ue(0)
                    b.u(index*2,4)
                    if index==0:b.u(0);b.u(0)
                    else:
                        b.u(0);b.u(0)
                        if weighted:
                            b.ue(1);b.ue(2);b.u(1);b.se(3);b.se(-5);b.u(1);b.se(3);b.se(7);b.se(-2);b.se(64)
                        b.u(0)
                        if cabac:b.ue(0)
                    b.se(0);b.ue(1)
                    if cabac:
                        while len(b.bits)%8:b.u(1)
                        c=CabacWriter(-1 if index==0 else 0,26)
                        if index==0:
                            c.decision(3,1);b.bits.extend(c.finish());b.align()
                            for p,(depth,w,bw)in enumerate([(yd,32,16),(cd,16,8),(cd,16,8)]):
                                for y in range(16):
                                    for x in range(bw):b.u(planes[p][y*w+address*bw+x],depth)
                            c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                        else:c.decision(11,1);c.range-=2;c.renormalize();c.decision(11,1);b.bits.extend(c.finish())
                    elif index==0:
                        for mb in range(2):
                            b.ue(25);b.align()
                            for p,(depth,w,bw)in enumerate([(yd,32,16),(cd,16,8),(cd,16,8)]):
                                for y in range(16):
                                    for x in range(bw):b.u(planes[p][y*w+mb*bw+x],depth)
                    elif mv==(0,0):b.ue(2)
                    else:
                        for mb in range(2):b.ue(0);b.ue(0);b.se(mv[0]if mb==0 else 0);b.se(mv[1]if mb==0 else 0);b.ue(0)
                    nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
                packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals);(OUT/f'{name}-{index}.packet').write_bytes(packet);frames.append((index,index==0,packet))
            predicted=[prediction(planes[0],yd,mv),chroma(planes[1],cd,mv),chroma(planes[2],cd,mv)]
            if weighted:
                for component,(depth,weight,offset,denom)in enumerate([(yd,3,-5,1),(cd,3,7,2),(cd,-2,64,2)]):
                    original=predicted[component]
                    predicted[component]=[max(0,min((1<<depth)-1,((weight*v+(1<<(denom-1)))>>denom)+(offset<<(depth-8))))for v in original]
                    assert predicted[component]!=original
            expected=sum(planes,[])+sum(predicted,[])
            for suffix,data in [('avcc',config),('mp4',mux(config,frames,32,16,25)),('y16',struct.pack('<2048H',*expected))]:(OUT/(name+'.'+suffix)).write_bytes(data)
            cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,motion=mv,weighted=weighted))
    cases.append(chroma_dc_case())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
def chroma_dc_case():
    # Complete owned P syntax with one signed chroma DC coefficient per component.
    base='422-p-y8-c8-e0-x0-y0';name='422-p-chroma-dc-pending'
    config=(OUT/(base+'.avcc')).read_bytes();first=(OUT/(base+'-0.packet')).read_bytes()
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
    for _ in range(2):
        b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(1);b.se(0) # inter CBP16
        for sign in [0,1]:b.u(1,2);b.u(sign);b.u(1) # 422 DC trailing one / zeros0
    nal=b.nal(0x41);second=len(nal).to_bytes(4,'big')+nal
    raw=(OUT/(base+'.y16')).read_bytes();samples=list(struct.unpack('<2048H',raw));target=samples[:1024]
    # qP_DC29, flat weight16, DC +/-1 gives dequantized DC +/-72.
    target[512:768]=[min(255,v+1)for v in target[512:768]];target[768:1024]=[max(0,v-1)for v in target[768:1024]]
    for suffix,data in [('avcc',config),('-0.packet',first),('-1.packet',second),('mp4',mux(config,[(0,True,first),(1,False,second)],32,16,25)),('y16',struct.pack('<2048H',*(samples[:1024]+target)))]:
        (OUT/(name+suffix if suffix.startswith('-')else name+'.'+suffix)).write_bytes(data)
    (OUT/'unsupported.json').write_text('[]\n')
    return dict(name=name,luma_depth=8,chroma_depth=8,cabac=False,motion=[0,0],weighted=False,residual='signed-chroma-dc')
if __name__=='__main__':generate()
