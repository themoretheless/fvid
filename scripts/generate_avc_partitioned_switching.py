"""Owned partitioned SP/SI streams and independent integer-matrix pixel oracle."""
import json
from pathlib import Path
from generate_avc_data_partitions import Writer, build, planes, packet, config, mux
from generate_avc_switching_luma_fixtures import reference
from generate_avc_switching_chroma_fixtures import chroma as primary_chroma
from generate_avc_secondary_sp_fixtures import chroma as secondary_chroma
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-switching'
def coded_residual(w):
    for block in range(16):w.u(1,2);w.u(block%2);w.u(1)
    for component in range(2):w.u(1);w.u(component);w.u(1,2)
    for component in range(2):
        for block in range(4):w.u(1,2);w.u((block+component)%2);w.u(1)
def slice_nals(frame,kind,qs,signed,addresses,partitioned):
    a=Writer();b=Writer() if partitioned else a;c=Writer() if partitioned else a
    sp=kind.startswith('sp');secondary=kind=='sp-secondary'
    a.ue(addresses[0]);a.ue(3 if sp else 4);a.ue(0);a.u(frame,4);a.u(frame*2,4);a.ue(0)
    if sp:a.u(0);a.u(0)
    a.u(0);a.se(0)
    if sp:a.u(int(secondary))
    a.se(qs-26);a.ue(1)
    if partitioned:
        a.ue(addresses[0])
        for stream in [b,c]:stream.ue(addresses[0]);stream.ue(0)
    used_b=used_c=False;after_skip=False
    for mb in addresses:
        if mb==1:
            if sp and not after_skip:a.ue(0)
            a.ue(30 if sp else 26);b.align();used_b=True
            for p,(width,bw,bh) in enumerate([(32,16,16),(16,8,8),(16,8,8)]):
                source=planes()[p]
                for y in range(bh):
                    for x in range(bw):b.u(source[y*width+bw+x],8)
        elif sp:
            if not signed:
                a.ue(1);after_skip=True
            else:
                a.ue(0);a.ue(0);a.se(0);a.se(0);a.ue(12);a.se(0)
                coded_residual(c);used_c=True
        else:
            a.ue(1 if kind=='si-ordinary' else 0)
            for _ in range(16):a.u(1)
            a.ue(0);a.ue(0 if signed else 3)
            if signed:a.se(0);coded_residual(b);used_b=True
    if not partitioned:return [a.nal(0x41)]
    return [a.nal(0x42)]+([b.nal(0x43)] if used_b else [])+([c.nal(0x44)] if used_c else [])
def reconstruct(previous,kind,qs,signed):
    current=planes();sp=kind.startswith('sp');secondary=kind=='sp-secondary';switching=kind!='si-ordinary'
    ready=set()
    for block in range(16):
        bx=(block&1)+((block>>2)&1)*2;by=((block>>1)&1)+(block>>3)*2;ox=bx*4;oy=by*4
        if sp:prediction=[previous[0][(oy+i//4)*32+ox+i%4] for i in range(16)]
        else:
            top=[current[0][(oy-1)*32+ox+x] for x in range(4)] if by and (bx,by-1) in ready else []
            left=[current[0][(oy+y)*32+ox-1] for y in range(4)] if bx and (bx-1,by) in ready else []
            neighbours=top+left;dc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 128
            prediction=[dc]*16
        levels=[(1 if block%2==0 else -1) if signed else 0]+[0]*15
        samples=reference(prediction,levels,26,qs,secondary if sp else True) if switching else prediction
        for i,v in enumerate(samples):current[0][(oy+i//4)*32+ox+i%4]=v
        ready.add((bx,by))
    for component in range(2):
        pred=[previous[component+1][y*16+x] for y in range(8) for x in range(8)] if sp else [128]*64
        dc=[0,0,(1 if component==0 else -1) if signed else 0,0]
        ac=[[0,(1 if (block+component)%2==0 else -1) if signed else 0]+[0]*14 for block in range(4)]
        chroma_qs=39 if qs==51 else qs
        if kind=='si-ordinary':samples=pred
        elif sp and not secondary:samples=primary_chroma(pred,dc,ac,26,chroma_qs)
        else:samples=secondary_chroma(pred,dc,ac,chroma_qs)
        for y in range(8):
            for x in range(8):current[component+1][y*16+x]=samples[y*8+x]
    return current
def generate():
    cfg=config();seed=packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False));files={'configuration.avcc':cfg,'seed.packet':seed};cases=[]
    for kind,signed in [('sp-primary',False),('sp-primary',True),('sp-secondary',False),('sp-secondary',True),('si-switching',False),('si-switching',True),('si-ordinary',False)]:
      for split in [False,True]:
        name=f'{kind}-signed{int(signed)}-split{int(split)}';records=[];frames=[(0,True,seed)];history=planes();gold=bytearray(sum(history,[]))
        for frame,qs in enumerate([0,26,51],1):
            actual=[];control=[]
            for group in ([[1],[0]] if split else [[0,1]]):
                actual+=slice_nals(frame,kind,qs,signed,group,True);control+=slice_nals(frame,kind,qs,signed,group,False)
            coded=packet(actual);files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-control.packet']=packet(control)
            records.append(dict(packet=f'{name}-{frame}.packet',control=f'{name}-{frame}-control.packet',qs=qs));frames.append((frame,False,coded))
            history=reconstruct(history,kind,qs,signed);gold.extend(sum(history,[]))
        files[name+'.mp4']=mux(cfg,frames,32,16,25);files[name+'.yuv']=bytes(gold)
        cases.append(dict(name=name,kind=kind,signed=signed,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned Extended SP/SI A/B/C; independent integer matrix luma/chroma switching oracle; QS0/26/51; no private media, external commands or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
