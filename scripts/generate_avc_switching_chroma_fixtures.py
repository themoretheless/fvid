#!/usr/bin/env python3
"""Own primary-SP chroma matrix oracle and complete SP-skip video."""
import json
from generate_avc_switching_luma_fixtures import DEST,C,Q,D,inverse,reference,video
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux

def chroma(p,dc,ac,qp,qs):
    matrices=[]
    for b in range(4):
        block=[p[(b//2*4+i//4)*8+b%2*4+i%4] for i in range(16)]
        matrices.append([sum(C[y][j]*block[j*4+k]*C[x][k] for j in range(4) for k in range(4)) for y in range(4) for x in range(4)])
    H=[[1,1],[1,-1]]
    def had(v):return [sum(H[y][j]*v[j*2+k]*H[x][k] for j in range(2) for k in range(2)) for y in range(2) for x in range(2)]
    def quant(v,cat,extra=0):
        n=(abs(v)*Q[qs%6][cat]+2**(14+qs//6+extra))//2**(15+qs//6+extra)
        return n if v>=0 else -n
    predicted=had([m[0] for m in matrices])
    qdc=[quant(predicted[i]+dc[i]*16*D[qp%6][0]*16*2**(qp//6)//512,0,1) for i in range(4)]
    scaled_dc=[v*16*D[qs%6][0]*2**(qs//6)//32 for v in had(qdc)]
    out=[0]*64
    for b in range(4):
        scaled=[scaled_dc[b]]
        for i in range(1,16):
            y,x=divmod(i,4);cat=0 if y%2==x%2==0 else (1 if y%2==x%2==1 else 2)
            v=matrices[b][i]+ac[b][i]*16*D[qp%6][cat]*[16,25,20][cat]*2**(qp//6)//1024
            scaled.append(quant(v,cat)*D[qs%6][cat]*2**(qs//6))
        samples=inverse(scaled)
        for i,v in enumerate(samples):out[(b//2*4+i//4)*8+b%2*4+i%4]=v
    return out

def coded_video():
    control=video(False);config=bytes.fromhex(control['configuration']);first=bytes.fromhex(control['packets'][0])
    frames=[(0,True,first)];packets=[first.hex()]
    pixels=bytes((i*13)%256 for i in range(384));gold=bytearray(pixels)
    for index,qs in enumerate([0,26,51],1):
        b=Writer();b.ue(0);b.ue(3);b.ue(0);b.u(index,4);b.u(index*2,4)
        b.u(0);b.u(0);b.u(0);b.se(0);b.u(0);b.se(qs-26);b.ue(1)
        b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(12);b.se(0)
        for block in range(16):b.u(1,2);b.u(block%2);b.u(1)
        for component in range(2):b.u(1);b.u(component);b.u(1,2)
        for component in range(2):
            for block in range(4):b.u(1,2);b.u((block+component)%2);b.u(1)
        nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal;packets.append(packet.hex());frames.append((index,False,packet))
        output=[0]*384
        for block in range(16):
            ox=block%4*4;oy=block//4*4;p=[pixels[(oy+i//4)*16+ox+i%4] for i in range(16)]
            reconstructed=reference(p,[1 if block%2==0 else -1]+[0]*15,26,qs,False)
            for i,v in enumerate(reconstructed):output[(oy+i//4)*16+ox+i%4]=v
        for component,offset in enumerate([256,320]):
            dc=[0,0,1 if component==0 else -1,0] # coded DC scan index 1 maps to raster (row=1,col=0)
            ac=[[0,1 if (block+component)%2==0 else -1]+[0]*14 for block in range(4)]
            output[offset:offset+64]=chroma(pixels[offset:offset+64],dc,ac,26,39 if qs==51 else qs)
        pixels=bytes(output);gold.extend(pixels)
    filename='avc-primary-sp-signed-residual-synthetic.mp4';reference_file='avc-primary-sp-signed-residual-reference.yuv'
    (DEST/filename).write_bytes(mux(config,frames,16,16,30));(DEST/reference_file).write_bytes(gold)
    return dict(configuration=config.hex(),packets=packets,file=filename,reference=reference_file)

def main():
    cases=[]
    for qp in [0,5,6,23,24,39]:
      for qs in range(40):
       for pattern in range(3):
        p=[128]*64 if pattern==0 else [((i*71+qs*17+qp*3)%256 if pattern==1 else (255 if i%3 else 0)) for i in range(64)]
        dc=[0]*4 if pattern==0 else [((i*11+qp+qs)%13)-6 for i in range(4)]
        ac=[[0 if i==0 or pattern==0 else ((i*19+b*5+qs+qp)%17)-8 for i in range(16)] for b in range(4)]
        cases.append(dict(qp=qp,qs=qs,prediction=p,dc=dc,ac=ac,expected=chroma(p,dc,ac,qp,qs)))
    control=video(False);config=bytes.fromhex(control['configuration']);first=bytes.fromhex(control['packets'][0]);frames=[(0,True,first)];packets=[first.hex()]
    pixels=bytes((i*13)%256 for i in range(384));gold=bytearray(pixels)
    chroma_qp=[*range(30),29,30,31,32,32,33,34,34,35,35,36,36,37,37,37,38,38,38,39,39,39,39]
    for index,qs in enumerate([0,26,51],1):
        b=Writer();b.ue(0);b.ue(3);b.ue(0);b.u(index,4);b.u(index*2,4)
        b.u(0);b.u(0);b.u(0);b.se(0);b.u(0);b.se(qs-26);b.ue(1);b.ue(1)
        nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal;packets.append(packet.hex());frames.append((index,False,packet))
        output=[0]*384
        for block in range(16):
            ox=(block%4)*4;oy=(block//4)*4;p=[pixels[(oy+i//4)*16+ox+i%4] for i in range(16)]
            reconstructed=reference(p,[0]*16,26,qs,False)
            for i,v in enumerate(reconstructed):output[(oy+i//4)*16+ox+i%4]=v
        for offset in [256,320]:output[offset:offset+64]=chroma(pixels[offset:offset+64],[0]*4,[[0]*16 for _ in range(4)],chroma_qp[26],chroma_qp[qs])
        pixels=bytes(output);gold.extend(pixels)
    (DEST/'avc-primary-sp-skip-synthetic.mp4').write_bytes(mux(config,frames,16,16,30))
    (DEST/'avc-primary-sp-skip-reference.yuv').write_bytes(gold)
    m=dict(cases=cases,video=dict(configuration=config.hex(),packets=packets,file='avc-primary-sp-skip-synthetic.mp4'),provenance='Own primary SP 4:2:0 chroma matrix and DC Hadamard oracle, H.264 8.6.1.2, mapped component QPs. Original gradient I_PCM then three zero-motion SP-skip pictures QSY 0/26/51 with independently reconstructed YUV. No private media, FFmpeg or foreign encoder; generation offline.')
    m['coded_video']=coded_video()
    (DEST/'avc-switching-chroma.json').write_text(json.dumps(m,separators=(',',':'))+'\n')
if __name__=='__main__':main()
