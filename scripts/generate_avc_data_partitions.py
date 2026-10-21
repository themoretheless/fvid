"""Owned Extended-profile A/B/C streams and independent scalar plane oracle."""
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_redundant_primary import config, packet
from avc_fixture_mp4 import mux
OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions'
def planes():
    return [[1+(x*11+y*17+p*41)%250 for y in range(h) for x in range(w)]
            for p,(w,h) in enumerate([(32,16),(16,8),(16,8)])]
def build(frame, kind, macroblocks, partitioned, first=0, sid=0, rank=0, pps=0, counter=True, delta=0, frame_num=None, poc=None, nri=None, direct_spatial=True):
    a=Writer(); b=Writer() if partitioned else a; c=Writer() if partitioned else a
    if nri is None: nri=3 if frame==0 else (0 if kind=='B' else 2)
    a.ue(first); a.ue({'I':2,'P':0,'B':1}[kind]); a.ue(pps); a.u(frame if frame_num is None else frame_num,4)
    if frame==0: a.ue(0)
    a.u(frame*2 if poc is None else poc,4)
    if counter: a.ue(rank)
    if kind=='B': a.u(int(direct_spatial))
    if kind in ['P','B']:
        a.u(0); a.u(0)
        if kind=='B': a.u(0)
    if nri:
        a.u(0)
        if frame==0: a.u(0)
    a.se(0); a.ue(1)
    if partitioned:
        a.ue(sid)
        for r in [b,c]:
            r.ue(sid)
            if counter: r.ue(rank)
    used_b=used_c=False
    previous_pcm=False
    pending_skip=0
    for mb,mode in macroblocks:
        if mode=='skip':
            assert kind=='B'
            pending_skip+=1
            continue
        if kind in ['P','B']:
            a.ue(pending_skip);pending_skip=0
        if mode=='pcm':
            a.ue({'I':25,'P':30,'B':48}[kind]); b.align(); used_b=True
            source=[[v+delta for v in component] for component in planes()]
            for p,(w,bw,bh) in enumerate([(32,16,16),(16,8,8),(16,8,8)]):
                for y in range(bh):
                    for x in range(bw): b.u(source[p][y*w+mb*bw+x],8)
        elif mode in ['inter','l0','l1','bi','direct']:
            code={'inter':1,'l0':1,'l1':2,'bi':3,'direct':0}[mode] if kind=='B' else 0
            a.ue(code)
            if kind!='B' or mode!='direct':
                a.se(0); a.se(0)
                if kind=='B' and mode=='bi': a.se(0); a.se(0)
            a.ue(2); a.se(0)
            # coeff_token uses nC=16 / 9 along a preceding PCM boundary.
            # Fixed-length tokens (nC>=8): (1,1)=000001, (0,0)=000011.
            c.u(1,6) if previous_pcm else c.u(1,2)
            c.u(0);c.u(3,3) # positive sign and total_zeros=1
            c.u(1)
            c.u(3,6) if previous_pcm else c.u(1)
            c.u(1)
            used_c=True
        elif mode=='intra4':
            a.ue({'I':0,'P':5,'B':23}[kind])
            for _ in range(16): a.u(1) # most probable DC modes
            a.ue(0); a.ue(29); a.se(0) # chroma DC, intra CBP mapping code29 =>1
            for value in [0,1,0,0,1,1,1,1,1]: b.u(value)
            used_b=True
        elif mode=='zero':
            a.ue(0); a.se(0); a.se(0); a.ue(0)
        else: raise ValueError(mode)
        previous_pcm = mode=='pcm'
    if pending_skip: a.ue(pending_skip)
    if not partitioned: return [a.nal((nri<<5)+(5 if frame==0 else 1))]
    return [a.nal((nri<<5)+2)]+([b.nal((nri<<5)+3)] if used_b else [])+([c.nal((nri<<5)+4)] if used_c else [])
def generate():
    files={'configuration.avcc':config(), 'seed.packet':packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False))}; cases=[]
    for kind,modes in [('I',['pcm','pcm']),('P',['inter','pcm']),('P',['pcm','inter']),('P',['inter','inter']),('P',['zero','zero']),('I',['intra4','pcm']),('P',['intra4','pcm'])]:
      for split in [False,True]:
        name=kind+'-'+'-'.join(modes)+'-split'+str(int(split)); frames=[(0,True,files['seed.packet'])]; manifest=[]
        # Non-IDR partitioned pictures follow an ordinary IDR initialization.
        for frame in [1,2]:
            groups=[[(i,modes[i])] for i in [1,0]] if split else [list(enumerate(modes))]
            nals=[]; control=[]
            for g in groups:
                sid=g[0][0] if split else 0
                nals+=build(frame,kind,g,True,g[0][0],sid)
                control+=build(frame,kind,g,False,g[0][0],sid)
            coded=packet(nals); clean=packet(control)
            files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-control.packet']=clean
            manifest.append(dict(packet=f'{name}-{frame}.packet',control=f'{name}-{frame}-control.packet'))
            frames.append((frame,False,coded))
        files[name+'.mp4']=mux(config(),frames,32,16,25)
        # Each inter frame adds literal integer-transform basis [4,2,-2,-4].
        oracle=[]
        for frame in range(3):
            values=planes()
            if kind=='P':
                for mb,mode in enumerate(modes):
                    if mode=='inter':
                        for y in range(4):
                            for x,d in enumerate([4,2,-2,-4]):
                                at=y*32+mb*16+x;values[0][at]=max(0,min(255,values[0][at]+d*frame))
            if frame and modes[0]=='intra4':
                # Literal DC prediction values in raster block order, derived
                # from already reconstructed top/left edge sums (8.3.1.2).
                dc=[[128,124,124,124],[128,126,125,125],[128,127,126,126],[128,128,127,127]]
                for y in range(16):
                    for x in range(16):
                        values[0][y*32+x]=dc[y//4][x//4]+([4,2,-2,-4][x] if x<4 and y<4 else 0)
                for component in [1,2]:
                    for y in range(8):
                        for x in range(8): values[component][y*16+x]=128
            oracle.extend(sum(values,[]))
        files[name+'.yuv']=bytes(oracle)
        cases.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',frames=manifest))
    # A0,A1,C1,B0 is permitted ASO (B/C order preserved per slice).
    a=build(1,'P',[(0,'pcm')],True,0,0);c=build(1,'P',[(1,'inter')],True,1,1)
    interleaved=packet([a[0],c[0],c[1],a[1]])
    files['interleaved.packet']=interleaved;files['interleaved-control.packet']=packet(build(1,'P',[(0,'pcm')],False)+build(1,'P',[(1,'inter')],False,1,1))
    base=build(1,'P',[(0,'inter'),(1,'pcm')],True)
    empty=Writer();empty.ue(0);empty.ue(0)
    wrong_id=Writer();wrong_id.ue(1);wrong_id.ue(0)
    wrong_rank=Writer();wrong_rank.ue(0);wrong_rank.ue(1)
    no_residual=build(1,'P',[(0,'zero'),(1,'zero')],True)
    bad=[]
    for name,nals,message in [
        ('orphan',[base[1]],'precedes partition A'),
        ('duplicate-b',base+[base[1]],'duplicate or out-of-order'),
        ('duplicate-c',base+[base[2]],'duplicate or out-of-order'),
        ('c-before-b',[base[0],base[2],base[1]],'duplicate or out-of-order'),
        ('missing-b',[base[0],base[2]],'bit'),
        ('missing-c',[base[0],base[1]],'bit'),
        ('reference-class',[base[0],bytes([3])+base[1][1:],base[2]],'reference class'),
        ('truncated-c',[base[0],base[1],base[2][:-1]],'bit'),
        ('duplicate-a',[base[0],base[0],base[1],base[2]],'duplicate partition A'),
        ('wrong-id',[base[0],wrong_id.nal(0x43)],'no matching partition A'),
        ('wrong-rank',[base[0],wrong_rank.nal(0x43)],'no matching partition A'),
        ('unneeded-c',no_residual+[empty.nal(0x44)],'without residual syntax'),
        ('truncated-b',[base[0],base[1][:-3],base[2]],'bit'),
    ]:
        files[name+'.packet']=packet(nals);bad.append(dict(file=name+'.packet',message=message))
    # Primary / redundant pictures may use different PPS counter presence.
    # Coded picture groups remain ordered by redundant_pic_cnt.
    cfg=config(); size=int.from_bytes(cfg[6:8],'big'); sps=cfg[8:8+size]; pp=[]
    for identifier in [0,1]:
        w=Writer();w.ue(identifier);w.ue(0);w.u(0);w.u(0);w.ue(0);w.ue(0);w.ue(0)
        w.u(0);w.u(0,2);w.se(0);w.se(0);w.se(0);w.u(1);w.u(0);w.u(identifier)
        pp.append(w.nal(0x68))
    files['mixed-counter.avcc']=cfg[:6]+len(sps).to_bytes(2,'big')+sps+bytes([2])+b''.join(len(p).to_bytes(2,'big')+p for p in pp)
    files['mixed-counter.packet']=packet(build(1,'I',[(0,'pcm'),(1,'pcm')],True,counter=False)+build(1,'I',[(0,'pcm'),(1,'pcm')],True,rank=1,pps=1))
    files['mixed-counter-seed.packet']=packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False,counter=False))
    mixed=[]
    for split in [False,True]:
      for ranks in [[1],[1,2,127]]:
        name=f'mixed-counter-split{int(split)}-ranks{len(ranks)}'; frames=[(0,True,files['mixed-counter-seed.packet'])]; records=[]
        for frame in [1,2]:
            groups=[[(1,'pcm')],[(0,'inter')]] if split else [[(0,'inter'),(1,'pcm')]]
            primary=[];control=[]
            for group in groups:
                sid=group[0][0] if split else 0
                primary+=build(frame,'P',group,True,first=group[0][0],sid=sid,counter=False)
                control+=build(frame,'P',group,False,first=group[0][0],counter=False)
            redundant=[]
            for rank in ranks:
                for group in ([[(1,'pcm')],[(0,'pcm')]] if split else [[(0,'pcm'),(1,'pcm')]]):
                    redundant+=build(frame,'I',group,True,first=group[0][0],sid=group[0][0] if split else 0,rank=rank,pps=1,delta=1)
            coded=packet(primary+redundant)
            files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-control.packet']=packet(control)
            records.append(dict(packet=f'{name}-{frame}.packet',control=f'{name}-{frame}-control.packet'))
            frames.append((frame,False,coded))
        files[name+'.mp4']=mux(files['mixed-counter.avcc'],frames,32,16,25)
        oracle=[]
        for frame in range(3):
            values=planes()
            for y in range(4):
                for x,d in enumerate([4,2,-2,-4]):
                    at=y*32+x;values[0][at]=max(0,min(255,values[0][at]+d*frame))
            oracle.extend(sum(values,[]))
        files[name+'.yuv']=bytes(oracle)
        mixed.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    primary=build(1,'I',[(0,'pcm'),(1,'pcm')],True,counter=False)
    red=build(1,'I',[(0,'pcm'),(1,'pcm')],True,rank=1,pps=1,delta=1)
    red2=build(1,'I',[(0,'pcm'),(1,'pcm')],True,rank=2,pps=1,delta=1)
    files['late-primary-b.packet']=packet([primary[0]]+red+[primary[1]])
    files['descending-rank.packet']=packet(primary+red2+red)
    files['late-primary-a.packet']=packet(primary+red+primary)
    for name in ['late-primary-b','descending-rank','late-primary-a']:
        files[name+'.mp4']=mux(files['mixed-counter.avcc'],[(0,True,files['mixed-counter-seed.packet']),(1,False,files[name+'.packet'])],32,16,25)
    files['manifest.json']=(json.dumps(dict(cases=cases,mixed_counter=mixed,invalid=bad,provenance='Owned Extended SPS/PPS; PCM coordinate oracle; inter inverse-transform literal basis; no private media/external commands/network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items(): (OUT/name).write_bytes(value)
