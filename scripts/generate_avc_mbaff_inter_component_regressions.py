#!/usr/bin/env python3
"""Authored MBAFF P/B component-precision controls and independent pixel oracles."""
import hashlib,json,struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer,CabacWriter
from avc_fixture_mp4 import mux
from generate_audio_resample_window_fixture import ebml
from generate_avc_component_residual import increment
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mbaff-inter-components'
def configuration(y,c,cabac):
    b=Writer()
    for v in [244,0,10]:b.u(v,8)
    b.ue(0);b.ue(1);b.ue(y-8);b.ue(c-8);b.u(0);b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(0);b.ue(0);b.u(0);b.u(1);b.u(1);b.u(0);b.u(0)
    sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+b'\1'+len(pps).to_bytes(2,'big')+pps

def header(kind,frame,poc,cabac,idr=False,reference=True,spatial=True):
    b=Writer();b.ue(0);b.ue({'I':2,'P':0,'B':1}[kind]);b.ue(0);b.u(frame,4);b.u(0)
    if idr:b.ue(0)
    b.u(poc,4)
    if kind=='B':b.u(int(spatial))
    if kind!='I':
        b.u(0);b.u(0)
        if kind=='B':b.u(0)
    if idr:b.u(0);b.u(0)
    elif reference:b.u(0)
    if cabac and kind!='I':b.ue(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
    return b

def intra(y,c,field,seed,cabac,poc):
    b=header('I',seed,poc,cabac,idr=seed==0);planes=[[0]*512,[0]*128,[0]*128]
    if cabac:entropy=CabacWriter(-1,26)
    for address in range(2):
        if cabac:
            if address==0:entropy.decision(70,int(field))
            entropy.decision(3+int(address==1 and not field),1);b.bits.extend(entropy.finish());b.align()
        else:
            if address==0:b.u(int(field))
            b.ue(25);b.align()
        for p,depth in enumerate([y,c,c]):
            side=16 if p==0 else 8
            for row in range(side):
                dest=row*2+address if field else row+address*side
                for col in range(side):
                    maximum=(1<<depth)-1;value=(row*73+col*31+p*43+address*19+seed*17)&maximum
                    if row==0 and col<4:value=[0,maximum,1,maximum-1][col]
                    b.u(value,depth);planes[p][dest*side+col]=value
        if cabac:entropy.low=0;entropy.range=510;entropy.width=9
    if cabac:b.bits.extend(entropy.finish())
    nal=b.nal(0x65 if seed==0 else 0x41,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal,planes

def inter(mode,field,cabac,frame=1,poc=2):
    bipred=mode.startswith('b-');direct=mode in ['b-spatial','b-temporal'];kind='B' if bipred else 'P';b=header(kind,2 if bipred else frame,poc,cabac,reference=not bipred,spatial=mode!='b-temporal')
    skip=mode=='p-skip'
    if not cabac:
        if skip:b.ue(2)
        else:
            for address in range(2):
                b.ue(0)
                if address==0:b.u(int(field))
                b.ue(0 if direct else 3 if bipred else 0)
                if field and not direct:
                    for _ in range(2 if bipred else 1):b.u(1)
                if not direct:
                    for _ in range(2 if bipred else 1):b.se(0);b.se(0)
                b.ue(19 if mode=='p-residual' else 0)
                if mode=='p-residual':
                    b.se(0)
                    for block in range(16):b.u(1,2);b.u(int(block%2==1));b.u(1)
                    for component in range(2):b.u(1);b.u(component);b.u(1)
    else:
        entropy=CabacWriter(0,26)
        for address in range(2):
            bottom=address==1 and not field
            entropy.decision((24 if bipred else 11)+(int(bottom) if not skip else 0),int(skip))
            if skip:continue
            if address==0:entropy.decision(70,int(field))
            if direct:entropy.decision(27,0)
            elif bipred:
                entropy.decision(27+int(bottom),1);entropy.decision(30,1);entropy.decision(31,0)
                for _ in range(3):entropy.decision(32,0)
            else:
                for context in [14,15,16]:entropy.decision(context,0)
            if field and not direct:
                for _ in range(2 if bipred else 1):entropy.decision(54,0)
            if not direct:
                for _ in range(2 if bipred else 1):
                    entropy.mvd(0,0);entropy.mvd(1,0)
            for context in ([75,76,75,76] if bottom else [73,74,75,76]):entropy.decision(context,0)
            entropy.decision(77,0)
        b.bits.extend(entropy.finish())
    nal=b.nal(0x01 if bipred else 0x41,trailing=not cabac)
    return len(nal).to_bytes(4,'big')+nal

def matroska(config,display,order,mode,timed=True):
    track=ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_MPEG4/ISO/AVC')+ebml('63a2',config)
    track+=(ebml('23e383',(20_000_000).to_bytes(4,'big')) if timed else b'')+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x20'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',len(display)*20.0))+ebml('4d80',b'fvid synthetic')+ebml('5741',b'fvid synthetic'))
    blocks=ebml('e7',b'\0')
    for index in order:
        group=ebml('a1',b'\x81'+struct.pack('>h',index*20)+b'\0'+display[index][0])+(ebml('9b',b'\x14') if timed else b'')
        if index:
            references=[-20,20] if mode.startswith('b-') and index==1 else [-40] if mode.startswith('b-') else [-20]
            for reference in references:group+=ebml('fb',reference.to_bytes(1,'big',signed=True))
        blocks+=ebml('a0',group)
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    return header+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',blocks))

def generate():
    files={};cases=[]
    pairs=[(8,8),(10,10),(12,12),(14,14),(8,10),(10,8),(8,14),(14,8),(12,14),(14,12)]
    for y,c in pairs:
        for field in [False,True]:
            for cabac in [False,True]:
                modes=['p-skip','p-zero','b-bi'] if cabac else ['p-skip','p-zero','b-bi','p-residual']
                if (y,c) in [(8,8),(8,14),(14,8)]:modes+=['b-spatial','b-temporal']
                for mode in modes:
                    name=f'y{y}-c{c}-'+('field' if field else 'frame')+'-'+('cabac' if cabac else 'cavlc')+'-'+mode
                    config=configuration(y,c,cabac);files[name+'.avcc']=config
                    first,a=intra(y,c,field,0,cabac,0);packet=inter(mode,field,cabac)
                    if mode.startswith('b-'):
                        future,z=intra(y,c,field,1,cabac,4)
                        mean=[[(v+w+1)>>1 for v,w in zip(left,right)] for left,right in zip(a,z)]
                        display=[(first,a),(packet,mean),(future,z)];order=[0,2,1]
                    elif mode=='p-residual':
                        reconstructed=[]
                        for p,(values,depth) in enumerate(zip(a,[y,c,c])):
                            width=16 if p==0 else 8;maximum=(1<<depth)-1
                            reconstructed.append([min(maximum,max(0,v+increment(26,depth,(1 if (i%width)//4%2==0 else -1) if p==0 else (1 if p==1 else -1),p!=0))) for i,v in enumerate(values)])
                        echo=inter('p-skip',field,cabac,frame=2,poc=4)
                        display=[(first,a),(packet,reconstructed),(echo,reconstructed)];order=[0,1,2]
                    else:display=[(first,a),(packet,a)];order=[0,1]
                    records=[]
                    for index,(encoded,planes) in enumerate(display):
                        file=f'{name}-{index}.packet';files[file]=encoded;record={'packet':file}
                        for p,values in zip(['y','cb','cr'],planes):
                            file=f'{name}-{index}.{p}';files[file]=b''.join(v.to_bytes(2,'little') for v in values);record[p]=file
                        records.append(record)
                    files[name+'.mp4']=mux(config,[(i,i==0,display[i][0]) for i in order],16,32,50)
                    cases.append({'file':name+'.mp4','configuration':name+'.avcc','luma_depth':y,'chroma_depth':c,'field':field,'cabac':cabac,'mode':mode,'frames':records,'decode_order':order})
                    if (y,c) in [(8,8),(8,14),(14,8)]:
                        files[name+'.mkv']=matroska(config,display,order,mode);cases[-1]['matroska']=name+'.mkv'

    return files,cases
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True);files,cases=generate()
    for name,data in files.items():(OUT/name).write_bytes(data)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'164 authored40/60ms MBAFF clips, independent component arrays. CAVLC/CABAC P-skip, coded zero-motion P, signed residual/reference echo and reordered explicit/spatial-direct/temporal-direct B frames. Skip pairs infer frame mode; field cases retain alternating reference samples. No external encoder/private media/network.','cases':cases,'files':{n:hashlib.sha256(d).hexdigest() for n,d in files.items()}},indent=2)+'\n')
