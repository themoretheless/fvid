"""Owned partitioned Extended-profile B pictures with two distinct references."""
import json
from pathlib import Path
from generate_avc_data_partitions import build, planes, packet, Writer, config, mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-b'
def configuration():
    # Same owned 32x16 SPS as the I/P matrix, but retain two reference pictures.
    w=Writer()
    for value in [88,0,30]:w.u(value,8)
    for value in [0,0,0,0,2]:w.ue(value)
    w.u(0);w.ue(1);w.ue(0);w.u(1);w.u(1);w.u(0);w.u(0)
    sps=w.nal(0x67);original=config();length=int.from_bytes(original[6:8],'big')
    return original[:6]+len(sps).to_bytes(2,'big')+sps+original[8+length:]
def oracle(modes):
    values=planes()
    for mb,mode in enumerate(modes):
        shift={'l0':0,'l1':4,'bi':2,'direct':2,'skip':2,'pcm':0,'intra4':0}[mode]
        for p,(width,bw,bh) in enumerate([(32,16,16),(16,8,8),(16,8,8)]):
            for y in range(bh):
                for x in range(bw): values[p][y*width+mb*bw+x]+=shift
        if mode in ['l0','l1','bi','direct']:
            for y in range(4):
                for x,d in enumerate([4,2,-2,-4]):
                    at=y*32+mb*16+x;values[0][at]=max(0,min(255,values[0][at]+d))
        if mode=='intra4':
            assert mb==0
            dc=[[128,124,124,124],[128,126,125,125],[128,127,126,126],[128,128,127,127]]
            for y in range(16):
                for x in range(16):values[0][y*32+x]=dc[y//4][x//4]+([4,2,-2,-4][x] if x<4 and y<4 else 0)
            for p in [1,2]:
                for y in range(8):
                    for x in range(8):values[p][y*16+x]=128
    return bytes(sum(values,[]))
def generate():
    cfg=configuration();files={'configuration.avcc':cfg};cases=[]
    seed=packet(build(0,'I',[(0,'pcm'),(1,'pcm')],False));future=packet(build(3,'I',[(0,'pcm'),(1,'pcm')],False,frame_num=1,delta=4));successor=packet(build(4,'P',[(0,'zero'),(1,'zero')],False,frame_num=2))
    files.update({'seed.packet':seed,'future.packet':future,'successor.packet':successor})
    base=bytes(sum(planes(),[]));later=bytes(v+4 for v in base)
    for modes in [('l0','pcm'),('pcm','l1'),('bi','bi'),('l0','l1'),('direct','pcm'),('skip','skip'),('skip','bi'),('intra4','pcm')]:
      for spatial in ([False,True] if 'direct' in modes or 'skip' in modes else [True]):
       for split in [False,True]:
        name='-'.join(modes)+f'-spatial{int(spatial)}-split{int(split)}';frames=[(0,True,seed),(3,False,future)];records=[dict(packet='seed.packet',control='seed.packet',presentation=0),dict(packet='future.packet',control='future.packet',presentation=3)]
        for frame in [1,2]:
            groups=[[(1,modes[1])],[(0,modes[0])]] if split else [list(enumerate(modes))]
            actual=[];control=[]
            for group in groups:
                kwargs=dict(frame_num=2,direct_spatial=spatial,first=group[0][0],sid=group[0][0] if split else 0)
                actual+=build(frame,'B',group,True,**kwargs);control+=build(frame,'B',group,False,**kwargs)
            coded=packet(actual);clean=packet(control)
            files[f'{name}-{frame}.packet']=coded;files[f'{name}-{frame}-control.packet']=clean
            frames.append((frame,False,coded));records.append(dict(packet=f'{name}-{frame}.packet',control=f'{name}-{frame}-control.packet',presentation=frame))
        frames.append((4,False,successor));records.append(dict(packet='successor.packet',control='successor.packet',presentation=4))
        files[name+'.mp4']=mux(cfg,frames,32,16,25);files[name+'.yuv']=base+oracle(modes)*2+later*2
        cases.append(dict(name=name,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned Extended two-reference B A/B/C streams; literal scalar prediction offsets and transform basis, no external tools/private media.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
