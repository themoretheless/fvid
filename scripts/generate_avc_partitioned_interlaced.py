"""Owned complementary-field and adaptive-frame A/B/C bitstreams and scalar pixels."""
import json
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_data_partitions import packet,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-data-partitions-interlaced'
def configuration(mbaff,fmo):
    w=Writer()
    for value in [88,0,10]:w.u(value,8)
    for value in [0,0,0,0,2]:w.ue(value)
    w.u(0);w.ue(1);w.ue(0);w.u(0);w.u(int(mbaff));w.u(1);w.u(0);w.u(0);sps=w.nal(0x67)
    w=Writer();w.ue(0);w.ue(0);w.u(0);w.u(0);w.ue(int(fmo))
    if fmo:w.ue(6);w.ue(1);w.u(0);w.u(1)
    w.ue(0);w.ue(0);w.u(0);w.u(0,2);w.se(0);w.se(0);w.se(0);w.u(1);w.u(0);w.u(0);pps=w.nal(0x68)
    return bytes([1,88,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def planes():return [[1+(x*11+y*17+p*41)%250 for y in range(h) for x in range(w)] for p,(w,h) in enumerate([(32,32),(16,16),(16,16)])]
def layout(address,component,mbaff,fields,bottom=False):
    bw=16 if component==0 else 8
    if mbaff:
        pair,parity=divmod(address,2);field=fields[pair]
        return pair*bw,parity if field else parity*bw,2 if field else 1,bw
    return address*bw,int(bottom),2,bw

def write_pcm(w,address,mbaff,fields,bottom):
    w.align()
    for p in range(3):
        x,y,step,bw=layout(address,p,mbaff,fields,bottom);width=32 if p==0 else 16;source=planes()[p]
        for row in range(bw):
            for col in range(bw):w.u(source[(y+row*step)*width+x+col],8)

def slice_nals(frame,first,mode,partitioned,mbaff,fields,split,bottom=False,field_index=0):
    a=Writer();b=Writer() if partitioned else a;c=Writer() if partitioned else a
    idr=frame==0 and (mbaff or field_index==0);intra=frame==0 or mode=='intra'
    a.ue(first);a.ue(2 if intra else 0);a.ue(0);a.u(frame,4);a.u(int(not mbaff))
    if not mbaff:a.u(int(bottom))
    if idr:a.ue(0)
    a.u(frame*2+field_index if not mbaff else frame*2,4)
    if not intra:
        a.u(0)
        if mbaff:a.u(0)
        else:
            # Select previous frame's same-parity field, even after this frame's
            # opposite-parity field has already become a reference.
            a.u(1);a.ue(0);a.ue(1);a.ue(3)
    a.u(0)
    if idr:a.u(0)
    a.se(0);a.ue(1)
    sid=first
    if partitioned:a.ue(sid);b.ue(sid);c.ue(sid)
    if mode=='allskip' and not intra:
        assert not mbaff and not split
        a.ue(2)
        return [a.nal(0x42 if partitioned else 0x41)]
    pairs=[first] if split else [0,1]
    used_b=used_c=False
    for pair in pairs:
        addresses=[pair*2,pair*2+1] if mbaff else [pair]
        for parity,address in enumerate(addresses):
            if not intra:a.ue(0)
            if mbaff and parity==0:a.u(int(fields[pair]))
            pcm=intra or mode=='pcm-left' and pair==0 or mode=='pcm-right' and pair==1
            if pcm:
                a.ue(25 if intra else 30);write_pcm(b,address,mbaff,fields,bottom);used_b=True
            else:
                a.ue(0)
                if mbaff and fields[pair]:a.u(1) # expanded ref_idx_l0=0
                a.se(0);a.se(0);a.ue(2);a.se(0);used_c=True
                left_pcm=pair==1 and mode=='pcm-left' and not split
                # Coded first 8x8: one +1/-1 at scan position1, three zero tokens.
                c.u(1,6) if left_pcm else c.u(1,2)
                c.u(int(pair==1));c.u(3,3)
                c.u(1)
                c.u(3,6) if left_pcm else c.u(1)
                c.u(1)
    if not partitioned:return [a.nal(0x65 if idr else 0x41)]
    return [a.nal(0x42)]+([b.nal(0x43)] if used_b else [])+([c.nal(0x44)] if used_c else [])

def update(previous,mode,mbaff,fields):
    if mode=='intra':return planes()
    out=[v[:] for v in previous];source=planes()
    for address in range(4 if mbaff else 2):
     for bottom in ([False] if mbaff else [False,True]):
        pair=address//2 if mbaff else address;pcm=mode=='pcm-left' and pair==0 or mode=='pcm-right' and pair==1
        if pcm:
            for p in range(3):
                x,y,step,bw=layout(address,p,mbaff,fields,bottom);width=32 if p==0 else 16
                for row in range(bw):
                    for col in range(bw):at=(y+row*step)*width+x+col;out[p][at]=source[p][at]
        else:
            x,y,step,_=layout(address,0,mbaff,fields,bottom);field=not mbaff or fields[pair]
            for row in range(4):
                for col in range(4):
                    at=(y+row*step)*32+x+col;delta=[4,2,-2,-4][row if field else col]*(1 if pair==0 else -1)
                    out[0][at]=max(0,min(255,out[0][at]+delta))
    return out

def make_packet(frame,mode,mbaff,fields,split,aso,bottom=False,field_index=0):
    groups=[1,0] if aso else [0,1]
    if not split:groups=[0]
    parts=[slice_nals(frame,first,mode,frame!=0,mbaff,fields,split,bottom,field_index) for first in groups]
    actual=[n for category in [2,3,4] for group in parts for n in group if n[0]&31==category] if aso and frame else [n for group in parts for n in group]
    control=[n for first in groups for n in slice_nals(frame,first,mode,False,mbaff,fields,split,bottom,field_index)]
    return packet(actual),packet(control)

def generate():
    files={};cases=[]
    for mbaff in [True,False]:
     for fmo in [False,True]:
      cfgname=('mbaff' if mbaff else 'paff')+f'-fmo{int(fmo)}.avcc';cfg=configuration(mbaff,fmo);files[cfgname]=cfg
      for split in ([True] if fmo else [False,True]):
       for aso in [False,True]:
        for mode in ['coded','pcm-left','pcm-right','intra']:
         for initial in ([False,True] if mbaff else [False]):
          for topology in ([(False,False),(True,True),(False,True),(True,False)] if mbaff else [(False,False),(True,True)]):
            bottom_first=topology[0] if not mbaff else False
            name=('mbaff' if mbaff else 'paff')+f'-fmo{int(fmo)}-split{int(split)}-aso{int(aso)}-{mode}-seed{int(initial)}-topology{int(topology[0])}{int(topology[1])}'
            frames=[];records=[];history=planes();gold=bytearray()
            for frame in range(3):
                fields=[initial]*2 if frame==0 else list(topology if frame==1 else [not v for v in topology])
                if frame:history=update(history,mode,mbaff,fields)
                gold.extend(sum(history,[]))
                for index,bottom in enumerate([False] if mbaff else ([True,False] if bottom_first else [False,True])):
                    actual,control=make_packet(frame,mode,mbaff,fields,split,aso,bottom,index)
                    stem=f'{name}-{frame}-{index}';files[stem+'.packet']=actual;files[stem+'-control.packet']=control
                    frames.append((frame if mbaff else frame*2+index,frame==0 and index==0,actual))
                    records.append(dict(packet=stem+'.packet',control=stem+'-control.packet',presentation=frame,emits=mbaff or index==1,fields=fields,bottom=bottom))
            files[name+'.mp4']=mux(cfg,frames,32,32,25 if mbaff else 50);files[name+'.yuv']=bytes(gold)
            cases.append(dict(name=name,mbaff=mbaff,fmo=fmo,split=split,aso=aso,mode=mode,configuration=cfgname,file=name+'.mp4',oracle=name+'.yuv',frames=records))
    def unpack(data):
        out=[];at=0
        while at<len(data):
            size=int.from_bytes(data[at:at+4],'big');at+=4;out.append(data[at:at+size]);at+=size
        return out
    prefix='paff-fmo0-split0-aso0-'
    for index,bottom in enumerate([False,True]):
        actual,control=make_packet(1,'allskip',False,[False,False],False,False,bottom,index)
        files[f'pure-skip-{index}.packet']=actual;files[f'pure-skip-{index}-control.packet']=control
    empty=Writer();empty.ue(0)
    for name,nals in [
        ('skip-extra-b',unpack(files['pure-skip-0.packet'])+[empty.nal(0x43)]),
        ('skip-extra-c',unpack(files['pure-skip-0.packet'])+[empty.nal(0x44)]),
        ('coded-missing-c',[n for n in unpack(files[prefix+'coded-seed0-topology00-1-0.packet']) if n[0]&31!=4]),
        ('pcm-missing-b',[n for n in unpack(files[prefix+'intra-seed0-topology00-1-0.packet']) if n[0]&31!=3]),
        ('pcm-extra-c',unpack(files[prefix+'intra-seed0-topology00-1-0.packet'])+[empty.nal(0x44)]),
    ]:
        files[name+'.packet']=packet(nals)
        seed=[(i,i==0,files[prefix+f'intra-seed0-topology00-0-{i}.packet']) for i in [0,1]]
        files[name+'.mp4']=mux(files['paff-fmo0.avcc'],seed+[(2,False,files[name+'.packet'])],32,32,50)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned Extended PAFF fields and MBAFF A/B/C, parity-distinct PCM, field/frame scan basis, scalar physical coordinates; no external/private data.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,value in generate().items():(OUT/name).write_bytes(value)
