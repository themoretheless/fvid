"""Owned nonseparate 4:4:4 CAVLC/CABAC intra fields, scan/precision/filter/parity witnesses."""
import itertools,json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,mux
from generate_avc_444_deblock import coefficient,chroma_qp,i16_samples,filtered
from generate_avc_separate_8x8_deblock import residual_8x8_single
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-fields'


def configuration(yd,cd,cabac,offsets,height_map_units=2,mbaff=False):
    b=Writer()
    for v in [244,0,41]:b.u(v,8)
    b.ue(0);b.ue(3);b.u(0);b.ue(yd-8);b.ue(cd-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(1);b.ue(height_map_units-1)
    b.u(0);b.u(int(mbaff));b.u(1);b.u(1)
    for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0);b.u(1);b.u(0);b.se(offsets[1]);pps=b.nal(0x68)
    return bytes([1,244,0,41,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps


def cabac_single(c,category,position,level):
    cbf=[85,89,93,97,101,1012,460,464,468,1016,472,476,480,1020][category]
    sig=[277,292,306,321,324,436,776,791,805,675,820,835,849,733][category]
    last=[338,353,367,382,385,451,864,879,893,699,908,923,937,757][category]
    absolute=[227,237,247,257,266,426,952,962,972,708,982,992,1002,766][category]
    c.decision(cbf+3,1)
    eight=category in [5,9,13]
    for pos in range(position+1):c.decision(sig+([0,1,1,2][pos]if eight else pos),int(pos==position))
    c.decision(last+(1 if eight and position else position),1)
    c.decision(absolute+1,int(abs(level)>1))
    if abs(level)>1:
        assert abs(level)==4
        for v in [1,1,0]:c.decision(absolute+5,v)
    c.bypass(int(level<0))


def predict4(depth,qp,sign):
    # Scan position 1 is raster (0,1) for fields, vertical frequency.
    prime=qp+6*(depth-8);v=sign*16*[13,14,16,18,20,23][prime%6]
    scaled=v<<(prime//6-4)if prime//6>=4 else(v+(1<<(3-prime//6)))>>(4-prime//6)
    residual=[scaled,scaled>>1,-(scaled>>1),-scaled]
    residual=[(v+32)>>6 for v in residual];plane=[0]*256
    for block in range(16):
        x=((block//4)%2*2+block%2)*4;y=((block//8)*2+(block%4)//2)*4;edge=[]
        if x:edge += [plane[(y+j)*16+x-1]for j in range(4)]
        if y:edge += plane[(y-1)*16+x:(y-1)*16+x+4]
        pred=(sum(edge)+len(edge)//2)//len(edge)if edge else 1<<(depth-1)
        for j in range(4):
            value=max(0,min((1<<depth)-1,pred+residual[j]))
            plane[(y+j)*16+x:(y+j)*16+x+4]=[value]*4
    return plane


def predict8(depth,qp,sign):
    # Field scan position 3 -> raster position 1 (frame scan would map to 16).
    residual=residual_8x8_single(1,4*sign,qp,depth,16);plane=[0]*256;ready=set()
    for block in range(4):
        x=block%2*8;y=block//2*8;edge=[]
        top=y>0 and (x//8,(y-1)//8)in ready
        left=x>0 and ((x-1)//8,y//8)in ready
        corner=x>0 and y>0 and ((x-1)//8,(y-1)//8)in ready
        c=plane[(y-1)*16+x-1]if corner else None
        if top:
            t=plane[(y-1)*16+x:(y-1)*16+x+8]
            t8=plane[(y-1)*16+x+8]if x+8<16 and ((x+8)//8,(y-1)//8)in ready else t[7]
            edge.append(((c if corner else t[0])+2*t[0]+t[1]+2)//4)
            edge.extend((t[i-1]+2*t[i]+t[i+1]+2)//4 for i in range(1,7));edge.append((t[6]+2*t[7]+t8+2)//4)
        if left:
            l=[plane[(y+i)*16+x-1]for i in range(8)]
            edge.append(((c if corner else l[0])+2*l[0]+l[1]+2)//4)
            edge.extend((l[i-1]+2*l[i]+l[i+1]+2)//4 for i in range(1,7));edge.append((l[6]+3*l[7]+2)//4)
        pred=(sum(edge)+len(edge)//2)//len(edge)if edge else 1<<(depth-1)
        for j in range(8):
            for i in range(8):plane[(y+j)*16+x+i]=max(0,min((1<<depth)-1,pred+residual[j*8+i]))
        ready.add((x//8,y//8))
    return plane


def field(yd,cd,cabac,offsets,mode,signal,bottom,index):
    nals=[];blocks=[[],[],[]];qps=[[],[],[]]
    for mb in range(4):
        b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.u(int(bottom is not None))
        if bottom is not None:b.u(int(bool(bottom)))
        if index==0:b.ue(0)
        b.u(index,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(6);b.se(6)
        pcm=signal=='pcm';eight=signal=='i8';nxn=signal in ['i4','i8']
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=Cabac444Writer();c.decision(3,int(not nxn))
            if pcm:b.bits.extend(c.finish());b.align()
            elif nxn:
                c.decision(399,int(eight))
                for _ in range(4 if eight else 16):c.decision(68,1)
                for _ in range(4):c.decision(73,1)
                c.decision(60,0)
            else:
                c.range-=2;c.renormalize()
                for ctx,v in [(6,0),(7,0),(9,1),(10,0),(60,0)]:c.decision(ctx,v)
        else:
            b.ue(25 if pcm else 0 if nxn else 3)
            if pcm:b.align()
            elif nxn:
                b.u(int(eight))
                for _ in range(4 if eight else 16):b.u(1)
                b.ue(0);b.se(0)
            else:b.se(0)
        for p,depth in enumerate([yd,cd,cd]):
            qp=(0 if pcm else 26)if p==0 else chroma_qp(0 if pcm else 26,offsets[p-1],depth);qps[p].append(qp)
            sign=-1 if (p+mb+int(bool(bottom)))%2 else 1
            if pcm:
                values=[(80+20*p+int(bool(bottom))*7+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(16)for x in range(16)]
                for value in values:b.u(value,depth)
            elif signal in ['dc','scan']:
                pos=p+1 if signal=='scan' else 0
                if cabac:
                    if bottom is not None:cabac_single(c,[0,6,10][p],pos,sign)
                    else:
                        assert pos==0
                        from generate_avc_444_cabac_intra import single
                        single(c,[0,6,10][p],3,sign)
                else:coefficient(b,sign,pos)
                values=i16_samples(depth,qp,sign,[0,2,1,3][pos])
            elif signal=='i4':
                for _ in range(16):
                    if cabac:cabac_single(c,[2,8,12][p],1,sign)
                    else:coefficient(b,sign,1)
                values=predict4(depth,qp,sign)
            else:
                if cabac:
                    for _ in range(4):cabac_single(c,[5,9,13][p],3,4*sign)
                else:
                    for sub in range(16):
                        if sub%4==3:coefficient(b,4*sign,0)
                        else:b.u(1)
                values=predict8(depth,qp,sign)
            blocks[p].append(values)
        if cabac:
            if pcm:c.low=0;c.range=510;c.width=9
            b.bits.extend(c.finish())
        nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac);nals.append(len(nal).to_bytes(4,'big')+nal)
    planes=[];frame_wrong=0
    for p,depth in enumerate([yd,cd,cd]):
        raw=[blocks[p][y//16*2+x//16][y%16*16+x%16]for y in range(32)for x in range(32)]
        correct=filtered(raw,depth,qps[p],mode,6,eight,field=bottom is not None)
        wrong=filtered(raw,depth,qps[p],mode,6,eight,field=False)
        frame_wrong+=sum(a!=b for a,b in zip(correct,wrong));planes.append(correct)
    return b''.join(nals),planes,frame_wrong


def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for (yd,cd),cabac,bottom_first,offsets,mode,signal in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[(0,0),(-12,12),(12,-12)],[0,1,2],['dc','scan','i4','i8','pcm']):
        name=f'{signal}-y{yd}-c{cd}-e{int(cabac)}-b{int(bottom_first)}-o{offsets[0]}-{offsets[1]}-f{mode}'
        cfg=configuration(yd,cd,cabac,offsets);packets=[];physical={}
        for index,bottom in enumerate([True,False]if bottom_first else[False,True]):
            packet,planes,count=field(yd,cd,cabac,offsets,mode,signal,bottom,index);packets.append(packet);physical[bottom]=planes;witness+=count
            (OUT/f'{name}-{index}.packet').write_bytes(packet)
        assert physical[False]!=physical[True]
        woven=[]
        for p in range(3):
            for y in range(32):
                for bottom in [False,True]:woven+=physical[bottom][p][y*32:(y+1)*32]
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,50)),('y16',struct.pack('<6144H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,pcm=signal=='pcm'))
    assert len(cases)==540 and witness>0
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases,wrong_frame_filter_witness=witness),indent=2)+'\n')
    print('field cases',len(cases),'wrong-frame-filter sample witnesses',witness)
if __name__=='__main__':generate()
