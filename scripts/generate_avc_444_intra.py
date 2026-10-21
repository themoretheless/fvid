"""Owned nonseparate 4:4:4 CAVLC intra samples; no external codec or private data."""
import json, struct, itertools
from pathlib import Path
from generate_avc_pcm_formats import Writer, configuration, mux
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-444-intra'

def generate():
    OUT.mkdir(exist_ok=True)
    cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        for mode in ['i4','i4-dc','i8','i16-zero','i16-dc']:
            for crop,joined in itertools.product([False,True],[False,True]):
                name=f'{mode}-y{yd}-c{cd}-crop{int(crop)}'+('-joined' if joined else '')
                cfg=configuration(3,yd,cd,crop,level=41)
                if mode=='i8':
                    n=int.from_bytes(cfg[6:8],'big'); prefix=cfg[:8+n]
                    p=Writer();p.ue(0);p.ue(0);p.u(0);p.u(0);p.ue(0);p.ue(0);p.ue(0);p.u(0);p.u(0,2)
                    p.se(0);p.se(0);p.se(0);p.u(1);p.u(0);p.u(0);p.u(1);p.u(0);p.se(0)
                    pps=p.nal(0x68);cfg=prefix+bytes([1])+len(pps).to_bytes(2,'big')+pps
                nals=[]; planes=[[],[],[]]
                for mb in range(2):
                    if mb==0 or not joined:
                        b=Writer(); b.ue(mb); b.ue(2); b.ue(0); b.u(0,4); b.ue(0); b.u(0,4); b.u(0); b.u(0); b.se(0); b.ue(1)
                    b.ue(0 if mode in ['i4','i4-dc','i8'] else 3)
                    if mode in ['i4','i4-dc','i8']:
                        if mode=='i8': b.u(1)
                        for _ in range(4 if mode=='i8' else 16): b.u(1)
                        b.ue(0 if mode=='i4-dc' else 1) # mono/444 coded_block_pattern=0
                    elif mode.startswith('i16'):
                        b.se(0)
                        for p in range(3):
                            if mode=='i16-dc':
                                b.u(1,2); b.u((p+mb)%2); b.u(1) # nC=0: one trailing one, zero zeros
                            else: b.u(1) # nC=0: TotalCoeff=0
                    if mode=='i4-dc':
                        b.se(0)
                        for p in range(3):
                            for _ in range(16): b.u(1,2); b.u((p+mb)%2); b.u(1)
                    if not joined or mb==1:
                        nal=b.nal(0x65); nals.append(len(nal).to_bytes(4,'big')+nal)
                    for p,depth in enumerate([yd,cd,cd]):
                        delta=0
                        if mode=='i16-dc':
                            qp=26+6*(depth-8); norm=[10,11,13,14,16,18][qp%6]*16
                            v=(-1 if (p+mb)%2 else 1)*norm
                            dc=v<<(qp//6-6) if qp//6>=6 else (v+(1<<(5-qp//6)))>>(6-qp//6)
                            delta=(dc+32)>>6
                        base=planes[p][-1][-1] if joined and mb and mode.startswith('i16') else 1<<(depth-1)
                        block=[max(0,min((1<<depth)-1,base+delta))]*256
                        if mode=='i4-dc':
                            qp=26+6*(depth-8); v=(-1 if (p+mb)%2 else 1)*[10,11,13,14,16,18][qp%6]*16
                            scaled=v<<(qp//6-4) if qp//6>=4 else (v+(1<<(3-qp//6)))>>(4-qp//6)
                            residual=(scaled+32)>>6
                            for i in range(16):
                                x=((i//4)%2*2+i%2)*4;y=((i//8)*2+(i%4)//2)*4
                                edges=[]
                                if x: edges += [block[(y+j)*16+x-1] for j in range(4)]
                                elif joined and mb: edges += [planes[p][-1][(y+j)*16+15] for j in range(4)]
                                if y: edges += block[(y-1)*16+x:(y-1)*16+x+4]
                                pred=(sum(edges)+len(edges)//2)//len(edges) if edges else 1<<(depth-1)
                                value=max(0,min((1<<depth)-1,pred+residual))
                                for j in range(4): block[(y+j)*16+x:(y+j)*16+x+4]=[value]*4
                        planes[p].append(block)
                packet=b''.join(nals)
                expected=[]
                for blocks in planes:
                    expected += [v for y in range(16) for block in blocks for v in block[y*16:y*16+16]]
                for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,16,25)),('y16',struct.pack('<1536H',*expected))]:
                    (OUT/f'{name}.{ext}').write_bytes(data)
                cases.append(dict(name=name,yd=yd,cd=cd,crop=[1]*4 if crop else [0]*4))
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n')
if __name__=='__main__': generate()
