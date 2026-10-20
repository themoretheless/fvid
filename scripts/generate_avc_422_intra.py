"""Owned CAVLC 422 intra prediction/DC residual witnesses, H.264 8.3.4/8.5.11."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-intra'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for mode in range(4):
            for sign in [-1,0,1]:
                for luma,ac in [(l,a)for l in ['i16','i4']for a in [False,True]]:
                    name=f'intra422-y{yd}-c{cd}-m{mode}-dc{sign}-{luma}'+('-ac'if ac else'');config=configuration(2,yd,cd,False,height_mbs=2)
                    planes=[[(x*17+y*23+c*43)&((1<<dep)-1) for y in range(32)for x in range(w)]for c,(dep,w)in enumerate([(yd,32),(cd,16),(cd,16)])]
                    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
                    for mb in range(3):
                        b.ue(25);b.align()
                        for p,(depth,stride,bw) in enumerate([(yd,32,16),(cd,16,8),(cd,16,8)]):
                            for y in range(16):
                                for x in range(bw):b.u(planes[p][(mb//2*16+y)*stride+mb%2*bw+x],depth)
                    b.ue((3+4*(2 if ac else int(sign!=0)))if luma=='i16' else 0)
                    if luma=='i4':
                        for _ in range(16):b.u(1)
                    b.ue(mode)
                    if luma=='i4':b.ue(41 if ac else 16 if sign else 3)
                    if luma=='i16' or sign or ac:b.se(0)
                    if luma=='i16':b.u(3,6)
                    if sign or ac:
                        for s in [sign,-sign]:
                            if s:b.u(1,2);b.u(int(s<0));b.u(1)
                            else:b.u(1)
                    if ac:
                        for component in range(2):
                            for block in range(8):
                                b.u(1,6 if block in [0,1,2,4,6] else 2)
                                b.u((component+block)%2);b.u(1) # AC1, total_zeros0
                    nal=b.nal(0x65);packet=len(nal).to_bytes(4,'big')+nal
                    # Luma reference uses separately reconstructed block boundaries.
                    if luma=='i16':
                        value=(sum(planes[0][15*32+16:16*32])+sum(planes[0][y*32+15]for y in range(16,32))+16)//32
                        for y in range(16,32):planes[0][y*32+16:y*32+32]=[value]*16
                    else:
                        for by in range(4):
                            for bx in range(4):
                                x,y=16+bx*4,16+by*4
                                edge=planes[0][(y-1)*32+x:(y-1)*32+x+4]+[planes[0][(y+j)*32+x-1]for j in range(4)]
                                value=(sum(edge)+4)//8
                                for j in range(4):planes[0][(y+j)*32+x:(y+j)*32+x+4]=[value]*4
                    for p in [1,2]:
                        t=planes[p][15*16+8:16*16];left=[planes[p][y*16+7]for y in range(16,32)];corner=planes[p][15*16+7]
                        h=sum(i*(t[3+i]-(corner if i==4 else t[3-i]))for i in range(1,5))
                        v=sum(i*(left[7+i]-(corner if i==8 else left[7-i]))for i in range(1,9))
                        qp=26+6*(cd-8)+3;value=sign*(1 if p==1 else -1)*18*16
                        dc=(value<<(qp//6-6))if qp//6>=6 else (value+(1<<(5-qp//6)))>>(6-qp//6)
                        residual=(dc+32)>>6
                        for y in range(16):
                            for x in range(8):
                                if mode==1:pred=left[y]
                                elif mode==2:pred=t[x]
                                elif mode==3:pred=(16*(t[7]+left[15])+((34*h+32)>>6)*(x-3)+((5*v+32)>>6)*(y-7)+16)>>5
                                else:
                                    ts=sum(t[x//4*4:x//4*4+4]);ls=sum(left[y//4*4:y//4*4+4])
                                    pred=(ts+2)//4 if x>=4 and y<4 else (ls+2)//4 if x<4 and y>=4 else (ts+ls+4)//8
                                pred=max(0,min((1<<cd)-1,pred))
                                block=(y//4)*2+x//4
                                ac_sign=1 if (p-1+block)%2==0 else -1
                                half_amplitude=ac_sign*8*(1<<((26+6*(cd-8))//6))
                                horizontal=[2,1,-1,-2][x%4]*half_amplitude
                                decoded_residual=(dc+horizontal+32)//64 if ac else residual
                                planes[p][(16+y)*16+8+x]=max(0,min((1<<cd)-1,pred+decoded_residual))
                    for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,32,25)),('y16',struct.pack('<2048H',*sum(planes,[])))]: (OUT/(name+'.'+suffix)).write_bytes(data)
                    cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,mode=mode,sign=sign,luma=luma,ac=ac))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
