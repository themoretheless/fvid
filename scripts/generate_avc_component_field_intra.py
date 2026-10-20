"""Own mono/422 compressed I fields with scalar compact prediction and field AC."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-component-field-intra'
def compact(fmt,yd,cd,bottom,mode,sign,luma,ac):
    planes=[[(x*17+y*23+p*43+int(bottom)*19)&((1<<d)-1) for y in range(32) for x in range(w)] if w else [] for p,(d,w) in enumerate([(yd,32),(cd,16 if fmt else 0),(cd,16 if fmt else 0)])]
    source=[p[:] for p in planes]
    if luma=='i16':
        value=(sum(planes[0][15*32+16:16*32])+sum(planes[0][y*32+15] for y in range(16,32))+16)//32
        for y in range(16,32):planes[0][y*32+16:y*32+32]=[value]*16
    else:
        for by in range(4):
            for bx in range(4):
                x,y=16+bx*4,16+by*4
                edge=planes[0][(y-1)*32+x:(y-1)*32+x+4]+[planes[0][(y+j)*32+x-1] for j in range(4)]
                value=(sum(edge)+4)//8
                for j in range(4):planes[0][(y+j)*32+x:(y+j)*32+x+4]=[value]*4
    scan_witnesses=0
    if fmt:
        for p in [1,2]:
            t=planes[p][15*16+8:16*16];left=[planes[p][y*16+7] for y in range(16,32)];corner=planes[p][15*16+7]
            h=sum(i*(t[3+i]-(corner if i==4 else t[3-i])) for i in range(1,5))
            v=sum(i*(left[7+i]-(corner if i==8 else left[7-i])) for i in range(1,9))
            qp=26+6*(cd-8)+3;value=sign*(1 if p==1 else -1)*18*16
            dc=(value<<(qp//6-6)) if qp//6>=6 else (value+(1<<(5-qp//6)))>>(6-qp//6)
            for y in range(16):
                for x in range(8):
                    if mode==1:pred=left[y]
                    elif mode==2:pred=t[x]
                    elif mode==3:pred=(16*(t[7]+left[15])+((34*h+32)>>6)*(x-3)+((5*v+32)>>6)*(y-7)+16)>>5
                    else:
                        ts=sum(t[x//4*4:x//4*4+4]);ls=sum(left[y//4*4:y//4*4+4])
                        pred=(ts+2)//4 if x>=4 and y<4 else (ls+2)//4 if x<4 and y>=4 else (ts+ls+4)//8
                    pred=max(0,min((1<<cd)-1,pred));block=y//4*2+x//4
                    amplitude=(1 if (p-1+block)%2==0 else -1)*8*(1<<((26+6*(cd-8))//6)) if ac else 0
                    # Field scan coefficient1 is raster(0,1), so its basis is vertical.
                    residual=(dc+[2,1,-1,-2][y%4]*amplitude+32)//64
                    decoded=max(0,min((1<<cd)-1,pred+residual))
                    if ac:
                        wrong=(dc+[2,1,-1,-2][x%4]*amplitude+32)//64
                        scan_witnesses+=int(decoded!=max(0,min((1<<cd)-1,pred+wrong)))
                    planes[p][(16+y)*16+8+x]=decoded
    if ac:assert scan_witnesses>0, 'field AC must differ from frame-scan output'
    return source,planes

def field(fmt,yd,cd,bottom,index,cabac,mode,sign,luma,ac):
    source,target=compact(fmt,yd,cd,bottom,mode,sign,luma,ac)
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
    if index==0:b.ue(0)
    b.u(index,4)
    if index==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(-1,26)
    for mb in range(3):
        if cabac:c.decision(3 if mb==0 else 4,1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for p,(stride,bw,depth) in enumerate([(32,16,yd),(16,8,cd),(16,8,cd)]):
            if fmt==0 and p:continue
            for y in range(16):
                for x in range(bw):b.u(source[p][(mb//2*16+y)*stride+mb%2*bw+x],depth)
        if cabac:c.low=0;c.range=510;c.width=9;c.range-=2;c.renormalize()
    pattern=(2 if ac else int(sign!=0)) if fmt else 0
    if not cabac:
        b.ue(3+4*pattern if luma=='i16' else 0)
        if luma=='i4':
            for _ in range(16):b.u(1)
        if fmt:b.ue(mode)
        if luma=='i4':b.ue((41 if ac else 16 if sign else 3) if fmt else 1)
        if luma=='i16' or pattern:b.se(0)
        if luma=='i16':b.u(3,6)
        if pattern:
            for s in [sign,-sign]:
                if s:b.u(1,2);b.u(int(s<0));b.u(1)
                else:b.u(1)
        if ac:
            for component in range(2):
                for block in range(8):b.u(1,6 if block in [0,1,2,4,6] else 2);b.u((component+block)%2);b.u(1)
    else:
        c.decision(5,int(luma=='i16'))
        if luma=='i16':
            c.range-=2;c.renormalize();c.decision(6,0);c.decision(7,int(pattern>0))
            if pattern:c.decision(8,int(pattern==2))
            c.decision(9,1);c.decision(10,0)
        else:
            for _ in range(16):c.decision(68,1)
        if fmt:
            c.decision(64,int(mode!=0))
            if mode:
                c.decision(67,int(mode>1))
                if mode>1:c.decision(67,int(mode==3))
        if luma=='i4':
            for ctx in [73,74,75,76]:c.decision(ctx,0)
            if fmt:
                c.decision(80,int(pattern>0))
                if pattern:c.decision(84,int(pattern==2))
        if luma=='i16' or pattern:c.decision(60,0)
        if luma=='i16':c.decision(88,0)
        if pattern:
            for s in [sign,-sign]:
                c.decision(100,int(s!=0))
                if s:c.decision(321,1);c.decision(382,1);c.decision(258,0);c.bypass(int(s<0))
        if ac:
            for component in range(2):
                for block in range(8):
                    c.decision(104,1);c.decision(324,1);c.decision(385,1);c.decision(267,0);c.bypass((component+block)%2)
        b.bits.extend(c.finish())
    nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal,target

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for fmt in [0,2]:
        for yd,cd in [(8,8),(10,12),(14,8)]:
            for cabac,reverse,luma in itertools.product([False,True],[False,True],['i16','i4']):
                for mode,sign,ac in (itertools.product(range(4),[-1,0,1],[False,True]) if fmt else [(0,0,False)]):
                    name=f'field-intra-f{fmt}-y{yd}-c{cd}-e{int(cabac)}-r{int(reverse)}-m{mode}-s{sign}-{luma}-ac{int(ac)}'
                    config=configuration(fmt,yd,cd,False,cabac,height_mbs=2);packets=[];fields={}
                    for index,bottom in enumerate([True,False] if reverse else [False,True]):
                        packet,planes=field(fmt,yd,cd,bottom,index,cabac,mode,sign,luma,ac);packets.append(packet);fields[bottom]=planes
                    assert fields[False]!=fields[True], "field parity must affect samples"
                    woven=[]
                    for p,w in enumerate([32,16,16]):
                        if fmt==0 and p:continue
                        for y in range(32):
                            for bottom in [False,True]:woven.extend(fields[bottom][p][y*w:(y+1)*w])
                    for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
                    for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,50)),('y16',struct.pack('<'+str(len(woven))+'H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
                    cases.append(dict(name=name,format=fmt,luma_depth=yd,chroma_depth=cd,cabac=cabac,bottom_first=reverse,mode=mode,sign=sign,luma=luma,ac=ac,crop=[0,0,0,0]))
    assert len(cases)==600
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
