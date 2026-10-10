#!/usr/bin/env python3
"""Independent matrix oracle for H.264 8.6 SP/SI luma, plus own SI video."""
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
DEST=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
C=[[1,1,1,1],[2,1,-1,-2],[1,-1,-1,1],[1,-2,2,-1]]
Q=[[13107,5243,8066],[11916,4660,7490],[10082,4194,6554],[9362,3647,5825],[8192,3355,5243],[7282,2893,4559]]
D=[[10,16,13],[11,18,14],[13,20,16],[14,23,18],[16,25,20],[18,29,23]]
def inverse(a):
    def line(v):
        e=[v[0]+v[2],v[0]-v[2],(v[1]//2)-v[3],v[1]+v[3]//2]
        return [e[0]+e[3],e[1]+e[2],e[1]-e[2],e[0]-e[3]]
    rows=[line(a[y*4:y*4+4]) for y in range(4)]
    cols=[line([rows[y][x] for y in range(4)]) for x in range(4)]
    return [max(0,min(255,(cols[x][y]+32)//64)) for y in range(4) for x in range(4)]
def reference(p,r,qp,qs,switch):
    transformed=[[sum(C[y][j]*p[j*4+k]*C[x][k] for j in range(4) for k in range(4)) for x in range(4)] for y in range(4)]
    scaled=[]
    for y in range(4):
      for x in range(4):
        cat=0 if y%2==x%2==0 else (1 if y%2==x%2==1 else 2)
        v=transformed[y][x]
        if not switch:v+=(r[y*4+x]*16*D[qp%6][cat]*[16,25,20][cat]*(2**(qp//6)))//1024
        quant=(abs(v)*Q[qs%6][cat]+2**(14+qs//6))//2**(15+qs//6)
        q=(quant if v>=0 else -quant)+(r[y*4+x] if switch else 0)
        scaled.append(q*D[qs%6][cat]*2**(qs//6))
    return inverse(scaled)
def video(si=True):
    b=Writer();b.u(88,8);b.u(0,8);b.u(10,8);b.ue(0);b.ue(0);b.ue(0);b.ue(0);b.ue(1);b.u(0);b.ue(0);b.ue(0);b.u(1);b.u(1);b.u(0);b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    config=bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps
    frames=[];packets=[]
    for i,qs in enumerate([0,26,51]):
        b=Writer();b.ue(0);b.ue(4 if si else 2);b.ue(0);b.u(i,4)
        if i==0:b.ue(0)
        b.u(i*2,4)
        if i==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0)
        if si:b.se(qs-26)
        b.ue(1)
        b.ue(26 if si else 25);b.align()
        for sample in range(384):b.u((sample*13+i*37)%256,8)
        nal=b.nal(0x65 if i==0 else 0x41);packet=len(nal).to_bytes(4,'big')+nal
        frames.append((i,i==0,packet));packets.append(packet.hex())
    filename='avc-si-pcm-synthetic.mp4' if si else 'avc-si-pcm-intra-control-synthetic.mp4'
    (DEST/filename).write_bytes(mux(config,frames,16,16,30))
    return dict(configuration=config.hex(),packets=packets,qs=[0,26,51],file=filename)
def main():
    cases=[]
    for qp in [0,5,6,23,24,51]:
      for qs in range(52):
       for switch in [False,True]:
        for pattern in range(2):
            p=[128]*16 if pattern==0 else [(i*71+qs*17+qp*3)%256 for i in range(16)]
            r=[0]*16 if pattern==0 else [((i*19+qs+qp)%17)-8 for i in range(16)]
            cases.append(dict(qp=qp,qs=qs,switching=switch,prediction=p,levels=r,expected=reference(p,r,qp,qs,switch)))
    (DEST/'avc-switching-luma.json').write_text(json.dumps(dict(cases=cases,video=video(),control=video(False),provenance='Own integer matrix forward transform, normative quantization and scalar inverse butterfly; H.264 8.6 equations 8-415..420 and 8-432..434. Own Extended-profile SI I_PCM video with deterministic gradient samples and matched I-slice control. No private media, FFmpeg, foreign encoder or network.'),separators=(',',':'))+'\n')
if __name__=='__main__':main()
