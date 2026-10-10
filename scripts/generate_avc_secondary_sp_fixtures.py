#!/usr/bin/env python3
"""Original secondary SP syntax and independent normative scalar YUV oracle."""
import json
from generate_avc_switching_luma_fixtures import DEST, C, Q, D, inverse, reference
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux


def chroma(p, dc, ac, qs):
    matrices=[]
    for b in range(4):
        block=[p[(b//2*4+i//4)*8+b%2*4+i%4] for i in range(16)]
        matrices.append([sum(C[y][j]*block[j*4+k]*C[x][k] for j in range(4) for k in range(4)) for y in range(4) for x in range(4)])
    H=[[1,1],[1,-1]]
    def had(v): return [sum(H[y][j]*v[j*2+k]*H[x][k] for j in range(2) for k in range(2)) for y in range(2) for x in range(2)]
    def quant(v,cat,extra=0):
        magnitude=abs(v)*Q[qs%6][cat]+2**(14+qs//6+extra)
        return ((-magnitude if v<0 else magnitude) if v else 0)//2**(15+qs//6+extra)
    predicted=had([m[0] for m in matrices])
    # H.264 8-439/440/441: quantize, add residual, Hadamard, copy DC.
    output_dc=had([quant(predicted[i],0,1)+dc[i] for i in range(4)])
    out=[0]*64
    for b in range(4):
        scaled=[output_dc[b]]
        for i in range(1,16):
            y,x=divmod(i,4);cat=0 if y%2==x%2==0 else (1 if y%2==x%2==1 else 2)
            scaled.append((quant(matrices[b][i],cat)+ac[b][i])*D[qs%6][cat]*2**(qs//6))
        for i,v in enumerate(inverse(scaled)): out[(b//2*4+i//4)*8+b%2*4+i%4]=v
    return out


def main():
    cases=[]
    for qs in range(40):
        for pattern in range(3):
            p=[128]*64 if pattern==0 else [(i*71+qs*17)%256 for i in range(64)]
            dc=[0]*4 if pattern==0 else [((i*11+qs)%13)-6 for i in range(4)]
            ac=[[0 if i==0 or pattern==0 else ((i*19+b*5+qs)%17)-8 for i in range(16)] for b in range(4)]
            cases.append(dict(qs=qs,prediction=p,dc=dc,ac=ac,expected=chroma(p,dc,ac,qs)))
    base=json.loads((DEST/'avc-switching-chroma.json').read_text())
    config=bytes.fromhex(base['video']['configuration'])
    first=bytes.fromhex(base['video']['packets'][0])
    videos=[]
    for coded in [False,True]:
        pixels=bytes((i*13)%256 for i in range(384));gold=bytearray(pixels)
        frames=[(0,True,first)];packets=[first.hex()]
        for index,qs in enumerate([0,26,51],1):
            b=Writer();b.ue(0);b.ue(3);b.ue(0);b.u(index,4);b.u(index*2,4)
            b.u(0);b.u(0);b.u(0);b.se(0);b.u(1);b.se(qs-26);b.ue(1)
            if coded:
                b.ue(0);b.ue(0);b.se(0);b.se(0);b.ue(12);b.se(0)
                for block in range(16):b.u(1,2);b.u(block%2);b.u(1)
                for component in range(2):b.u(1);b.u(component);b.u(1,2)
                for component in range(2):
                    for block in range(4):b.u(1,2);b.u((block+component)%2);b.u(1)
            else:b.ue(1)
            nal=b.nal(0x41);packet=len(nal).to_bytes(4,'big')+nal;packets.append(packet.hex());frames.append((index,False,packet))
            out=[0]*384
            for block in range(16):
                ox=block%4*4;oy=block//4*4;p=[pixels[(oy+i//4)*16+ox+i%4] for i in range(16)]
                levels=[(1 if block%2==0 else -1) if coded else 0]+[0]*15
                samples=reference(p,levels,26,qs,True)
                for i,v in enumerate(samples):out[(oy+i//4)*16+ox+i%4]=v
            for component,offset in enumerate([256,320]):
                dc=[0,0,(1 if component==0 else -1) if coded else 0,0]
                ac=[[0,(1 if (block+component)%2==0 else -1) if coded else 0]+[0]*14 for block in range(4)]
                out[offset:offset+64]=chroma(pixels[offset:offset+64],dc,ac,39 if qs==51 else qs)
            pixels=bytes(out);gold.extend(pixels)
        name='avc-secondary-sp-'+('signed' if coded else 'skip')
        (DEST/(name+'-synthetic.mp4')).write_bytes(mux(config,frames,16,16,30))
        (DEST/(name+'-reference.yuv')).write_bytes(gold)
        videos.append(dict(configuration=config.hex(),packets=packets,file=name+'-synthetic.mp4',reference=name+'-reference.yuv'))
    (DEST/'avc-secondary-sp.json').write_text(json.dumps(dict(cases=cases,videos=videos,provenance='Original Extended-profile secondary SP streams; independent integer matrix oracle follows H.264 8.6.2, specifically unscaled chroma DC copy in 8-441. JM19 scales that DC differently and is not the pixel oracle for these streams. Signed residual syntax mirrors original primary SP CAVLC fixtures; no private media, external decoder, FFmpeg or network.'),separators=(',',':'))+'\n')
if __name__=='__main__':main()
