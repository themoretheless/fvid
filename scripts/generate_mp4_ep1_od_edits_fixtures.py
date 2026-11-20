"""Own LD-SBR file OD/edit-clock qualification. No processes or network."""
import json
from generate_mp4_ep1_od_fixtures import OUT as BASE,author,fragmented
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_fixtures import SOURCE,word
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-od-edits'
def edited(source,n,repeated=True,variant=None,od_replays=False):
    windows=[(n//2,None),(6*n,n//2),(6*n,3*n)]if repeated else[(12*n,0)]
    if od_replays:windows=[(n//2,None),(12*n,0),(12*n,0)]
    total=sum(d for d,_ in windows)
    oldscale=int.from_bytes(dict(boxes(dict(boxes(source))[b'moov']))[b'mvhd'][12:16],'big')
    root_id=None
    for t,p in boxes(dict(boxes(source))[b'moov']):
        if t!=b'trak':continue
        parts=dict(boxes(p));md=dict(boxes(parts[b'mdia']))
        if md[b'hdlr'][8:12]==b'soun'and b'dpnd'not in dict(boxes(parts.get(b'tref',b''))):root_id=int.from_bytes(parts[b'tkhd'][12:16],'big')
    assert root_id is not None
    def track(body):
        items=dict(boxes(body));ident=int.from_bytes(items[b'tkhd'][12:16],'big')
        if ident not in [root_id,100]:return b''.join(box(t,p[:20]+word(int.from_bytes(p[20:24],'big')*24000//oldscale)+p[24:])if t==b'tkhd'else box(t,p)for t,p in boxes(body))
        od=ident==100;rows=[]
        for index,(duration,start)in enumerate(windows):
            media=-1 if start is None else start*(3 if od else 1)+(1 if od else 0)
            if od and od_replays and index==2:media+=36*n
            if od and variant=='empty'and index==1:media=-1
            if od and variant=='divergent'and index==2:media+=1
            if od and variant=='short'and index==len(windows)-1:duration-=1
            if od and index==1 and media>=0:
                rows.extend([(2*n,media),(duration-2*n,media+6*n)])
            else:rows.append((duration,media))
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:
                p=b''.join(rewrite(k,v)for k,v in boxes(p)if not(od and k==b'ctts'))
                if t==b'stbl'and od:
                    count=int.from_bytes(dict(boxes(p))[b'stsz'][8:12],'big')
                    if count:p+=box(b'ctts',word(0,1,count,1))
            elif t==b'tkhd':p=p[:20]+word(total)+p[24:]
            elif t==b'mdhd'and od:p=p[:16]+word(int.from_bytes(p[16:20],'big')+1)+p[20:]
            return box(t,p)
        payload=b''.join(rewrite(t,p)for t,p in boxes(body)if t!=b'edts')
        if ident==root_id and not repeated:return payload
        if od and variant=='root-only':return payload
        elst=word(0,len(rows))+b''.join(word(d,s&0xffffffff,0x10000)for d,s in rows)
        return payload+box(b'edts',box(b'elst',elst))
    def movie(body):
        return b''.join(box(t,track(p)if t==b'trak'else p[:12]+word(24000,total)+p[20:]if t==b'mvhd'else p)for t,p in boxes(body))
    def fragment(body):
        out=[]
        for t,p in boxes(body):
            if t==b'traf':
                children=dict(boxes(p));ident=int.from_bytes(children[b'tfhd'][4:8],'big')
                if ident==100:
                    p=b''.join(box(k,v[:4]+(int.from_bytes(v[4:],'big')+1).to_bytes(len(v)-4,'big'))if k==b'tfdt'else box(k,v)for k,v in boxes(p))
            out.append(box(t,p))
        return b''.join(out)
    return b''.join(box(t,movie(p)if t==b'moov'else fragment(p)if t==b'moof'else p)for t,p in boxes(source))
def generate():
    files={};cases=[];bad=[]
    for n in [480,512]:
      for cfg in [1,7]:
        control,golden,width,channels=eld_control(n,cfg,frame_count=12);stem=f'n{n}-c{cfg}';stride=channels*4
        pcm=bytes(n*stride)+golden[n*stride:13*n*stride]+golden[6*n*stride:18*n*stride]
        files[stem+'.f32le']=pcm
        files[stem+'-repeat-control.mp4']=edited(control,n)
        files[stem+'-plain-control.mp4']=control
        for form in ['snapshots','deltas','replace']:
          for unused in [False,True]:
            raw,_,_=author(control,n,form,unused=unused)
            for mode in ['indexed','fragmented','hybrid']:
                name=f'{stem}-{form}'+('-unused'if unused else'')+'-'+mode;video=name+'.mp4'
                source=raw if mode=='indexed'else fragmented(raw,n,mode=='hybrid')
                files[video]=edited(source,n)
                cases.append(dict(name=name,video=video,control=stem+'-repeat-control.mp4',golden=stem+'.f32le',n=n,rate=48000,channels=channels,frames=12,repeat=True,mode=mode))
        raw,_,_=author(control,n,'deltas');video=stem+'-shift-only.mp4';files[video]=edited(raw,n,False)
        cases.append(dict(name=stem+'-shift-only',video=video,control=stem+'-plain-control.mp4',golden=f'../aac-eld-esc-sbr/n{n}-c{cfg}-x2-crc1.f32le',n=n,rate=48000,channels=channels,frames=12,repeat=False,mode='indexed'))
        for variant,error in [('empty','MP4 OD empty edit overlaps active audio'),('short','MP4 OD edit leaves active audio uncovered'),('divergent','MP4 OD edits require presentation-dependent membership'),('root-only','MP4 OD edits require presentation-dependent membership')]:
            video=stem+'-'+variant+'.mp4';files[video]=edited(raw,n,True,variant)
            bad.append(dict(video=video,error=error))
    assert len(cases)==76 and len(bad)==16
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own 12-frame LD-SBR core/CLDFB PCM. Exact MP4 root/OD edit relation, one-OD-tick CTS offset, initial silence, fractional-frame crop, repeated source windows and differently partitioned OD edits; indexed/fragmented/hybrid storage. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
