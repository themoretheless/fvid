"""Own monochrome CAVLC PCM-to-intra prediction witnesses; no external codec."""
import json, struct
from generate_avc_pcm_formats import configuration, Writer, mux
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-monochrome-intra'
def generate():
    OUT.mkdir(exist_ok=True); cases=[]
    for depth in [8,10,14]:
        for mode in ['i16-horizontal','i4-dc']:
            name=f'mono-{depth}-{mode}'; config=configuration(0,depth,8,False)
            pcm=[(x*17+y*23+9)% (1<<depth) for y in range(16) for x in range(16)]
            b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
            b.ue(25);b.align()
            for value in pcm:b.u(value,depth)
            if mode=='i16-horizontal':
                b.ue(2);b.se(0);b.u(3,6) # nC=16 zero coeff_token
                right=[pcm[y*16+15] for y in range(16)]
            else:
                b.ue(0)
                for _ in range(16):b.u(1) # predicted DC modes
                b.ue(1) # monochrome CBP=0; no chroma prediction syntax
                right=[(sum(pcm[(y//4*4+j)*16+15] for j in range(4))+2)//4 for y in range(16)]
            expected=[v for y in range(16) for v in pcm[y*16:y*16+16]+[right[y]]*16]
            if mode=='i4-dc':
                # Each later block sees both reconstructed top and left edges.
                for by in range(4):
                    for bx in range(4):
                        x,y=16+bx*4,by*4
                        edge=[expected[(y+j)*32+x-1] for j in range(4)]
                        if y:edge += expected[(y-1)*32+x:(y-1)*32+x+4]
                        value=(sum(edge)+len(edge)//2)//len(edge)
                        for j in range(4):expected[(y+j)*32+x:(y+j)*32+x+4]=[value]*4
            nal=b.nal(0x65);packet=len(nal).to_bytes(4,'big')+nal
            (OUT/(name+'.avcc')).write_bytes(config);(OUT/(name+'.packet')).write_bytes(packet)
            (OUT/(name+'.mp4')).write_bytes(mux(config,[(0,True,packet)],32,16,25))
            (OUT/(name+'.y16')).write_bytes(struct.pack('<512H',*expected));cases.append(dict(name=name,depth=depth,mode=mode))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
